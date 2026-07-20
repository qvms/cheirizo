//! Session supervision monitor task.
//!
//! Aggregates runtime subsystem events into a unified `SessionStatus` and
//! broadcasts updates through `tokio::sync::watch`.

use tokio::sync::{mpsc, watch};
use tracing::{debug, info};

use super::{SessionStatus, SessionStatusEvent, SessionStatusReporter, SessionStatusSubscriber};

/// Central session supervisor that aggregates subsystem events.
///
/// Create via `SessionSupervisor::new()`, which returns the monitor
/// plus a `SessionStatusReporter` (for subsystems to send events) and a
/// `SessionStatusSubscriber` (for subsystems to read current health).
pub struct SessionSupervisor {
    /// Receives status events from subsystems
    event_rx: mpsc::UnboundedReceiver<SessionStatusEvent>,
    /// Broadcasts aggregated session status
    state_tx: watch::Sender<SessionStatus>,
    /// Shutdown signal
    shutdown: tokio::sync::broadcast::Receiver<()>,
}

impl SessionSupervisor {
    /// Create a new session supervisor with reporter and subscriber handles.
    ///
    /// The returned `SessionStatusReporter` should be cloned and distributed to
    /// subsystems that need to report status events.
    ///
    /// The returned `SessionStatusSubscriber` should be cloned and distributed to
    /// subsystems that need to read session status.
    pub fn new(
        shutdown: tokio::sync::broadcast::Receiver<()>,
    ) -> (Self, SessionStatusReporter, SessionStatusSubscriber) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (state_tx, state_rx) = watch::channel(SessionStatus::default());

        let reporter = SessionStatusReporter { tx: event_tx };
        let subscriber = SessionStatusSubscriber { rx: state_rx };

        let monitor = Self {
            event_rx,
            state_tx,
            shutdown,
        };

        (monitor, reporter, subscriber)
    }

    /// Run the session supervisor event loop.
    ///
    /// Consumes self. Runs until shutdown signal or all reporters are dropped.
    pub async fn run(mut self) {
        info!("Session supervisor started");

        loop {
            let event = tokio::select! {
                Some(event) = self.event_rx.recv() => event,
                _ = self.shutdown.recv() => {
                    info!("Session supervisor received shutdown");
                    break;
                }
            };

            self.handle_event(event);
        }

        info!("Session supervisor stopped");
    }

    fn handle_event(&self, event: SessionStatusEvent) {
        debug!("Health event: {event:?}");

        self.state_tx.send_modify(|state| {
            let (old_overall, new_overall) = state.apply_event(event);
            if new_overall != old_overall {
                info!("Session health changed: {} → {}", old_overall, new_overall);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdp::session::supervision::{GraphicsStreamState, OverallStatus};

    #[tokio::test]
    async fn test_monitor_session_closed() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (monitor, reporter, subscriber) = SessionSupervisor::new(shutdown_rx);

        let monitor_handle = tokio::spawn(monitor.run());

        reporter.report(SessionStatusEvent::SessionClosed {
            reason: "compositor destroyed session".into(),
        });

        // Small yield to let the monitor process
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        let state = subscriber.current();
        assert_eq!(state.overall, OverallStatus::Invalid);
        assert!(state.session.is_failed());
        assert!(!subscriber.is_session_valid());

        let _ = shutdown_tx.send(());
        let _ = monitor_handle.await;
    }

    #[tokio::test]
    async fn test_monitor_session_invalidated_cascades() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (monitor, reporter, subscriber) = SessionSupervisor::new(shutdown_rx);

        let monitor_handle = tokio::spawn(monitor.run());

        reporter.report(SessionStatusEvent::SessionInvalidated {
            reason: "D-Bus: non-existing session".into(),
        });

        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        let state = subscriber.current();
        assert_eq!(state.overall, OverallStatus::Invalid);
        assert!(state.session.is_failed());
        assert!(state.input.is_failed());
        assert!(state.clipboard.is_failed());
        assert!(!subscriber.is_session_valid());

        let _ = shutdown_tx.send(());
        let _ = monitor_handle.await;
    }

    #[tokio::test]
    async fn test_monitor_graphics_paused_degrades() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (monitor, reporter, subscriber) = SessionSupervisor::new(shutdown_rx);

        let monitor_handle = tokio::spawn(monitor.run());

        reporter.report(SessionStatusEvent::GraphicsStreamStateChanged {
            state: GraphicsStreamState::Paused,
        });

        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        let state = subscriber.current();
        assert_eq!(state.overall, OverallStatus::Degraded);
        assert!(!state.graphics.is_healthy());
        // Session is still valid even when degraded
        assert!(subscriber.is_session_valid());

        let _ = shutdown_tx.send(());
        let _ = monitor_handle.await;
    }

    #[tokio::test]
    async fn test_monitor_recovery() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (monitor, reporter, subscriber) = SessionSupervisor::new(shutdown_rx);

        let monitor_handle = tokio::spawn(monitor.run());

        // Degrade then recover
        reporter.report(SessionStatusEvent::InputFailed {
            reason: "transient".into(),
            permanent: false,
        });
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert_eq!(subscriber.current().overall, OverallStatus::Degraded);

        reporter.report(SessionStatusEvent::InputRecovered);
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert_eq!(subscriber.current().overall, OverallStatus::Healthy);

        let _ = shutdown_tx.send(());
        let _ = monitor_handle.await;
    }

    #[tokio::test]
    async fn test_monitor_graphics_stall_and_resume() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (monitor, reporter, subscriber) = SessionSupervisor::new(shutdown_rx);

        let monitor_handle = tokio::spawn(monitor.run());

        // Report stall
        reporter.report(SessionStatusEvent::VideoFrameStalled {
            stall_duration_ms: 5000,
        });
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        let state = subscriber.current();
        assert_eq!(state.overall, OverallStatus::Degraded);
        assert!(!state.graphics.is_healthy());
        // Stall is degraded, not failed — session stays valid
        assert!(subscriber.is_session_valid());

        // Report resume
        reporter.report(SessionStatusEvent::VideoFrameResumed);
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert_eq!(subscriber.current().overall, OverallStatus::Healthy);
        assert!(subscriber.current().graphics.is_healthy());

        let _ = shutdown_tx.send(());
        let _ = monitor_handle.await;
    }
}
