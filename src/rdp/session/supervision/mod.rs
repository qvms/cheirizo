//! Session supervision and status aggregation.
//!
//! Provides runtime liveness monitoring for session aspects (graphics, input,
//! clipboard, session lifecycle). Complements startup capability detection by tracking whether those capabilities remain functional at runtime.
//!
//! # Architecture
//!
//! ```text
//! Session/stream events
//!   Portal Closed ──────┐
//!   EIS stream EOF ─────┤     ┌──────────────────┐
//!                       ├────▶│ Session Supervisor │──▶ watch<SessionStatus>
//!   PipeWire state ─────┤     │  Task             │
//!   Input errors ───────┤     │                  │
//!   Clipboard errors ───┘     └──────────────────┘
//! ```
//!
//! Two primitives:
//! - `tokio::sync::watch<SessionStatus>` — broadcasts status to subscribers
//! - `tokio::sync::mpsc<SessionStatusEvent>` — aspects report events to the supervisor

use std::fmt;

use tracing::{debug, error, info, warn};

mod monitor;

pub use monitor::SessionSupervisor;

/// Stable identifiers for status-producing session aspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionAspectId {
    /// Managed compositor session lifecycle.
    Session,
    /// Graphics capture and encoding pipeline.
    Graphics,
    /// Input injection path.
    Input,
    /// Clipboard provider path.
    Clipboard,
}

impl fmt::Display for SessionAspectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session => write!(f, "session"),
            Self::Graphics => write!(f, "graphics"),
            Self::Input => write!(f, "input"),
            Self::Clipboard => write!(f, "clipboard"),
        }
    }
}

/// Normalized event emitted when a session aspect's health changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AspectHealthChanged {
    pub aspect: SessionAspectId,
    pub health: AspectHealth,
}

/// Health of a single session aspect
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AspectHealth {
    /// Operating normally
    Healthy,
    /// Impaired but functional (e.g., PipeWire paused, clipboard degraded)
    Degraded(String),
    /// Not working — recovery may be possible
    Failed(String),
    /// Not applicable in this session configuration (e.g., clipboard on view-only)
    NotApplicable,
}

impl AspectHealth {
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy | Self::NotApplicable)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

impl fmt::Display for AspectHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded(reason) => write!(f, "degraded: {reason}"),
            Self::Failed(reason) => write!(f, "failed: {reason}"),
            Self::NotApplicable => write!(f, "not applicable"),
        }
    }
}

/// Aggregated session status
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStatus {
    /// PipeWire stream liveness
    pub graphics: AspectHealth,
    /// Input injection liveness (Portal D-Bus / EIS / Wayland)
    pub input: AspectHealth,
    /// Clipboard provider liveness
    pub clipboard: AspectHealth,
    /// Managed compositor session object validity
    pub session: AspectHealth,
    /// Computed from individual subsystems
    pub overall: OverallStatus,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self {
            graphics: AspectHealth::Healthy,
            input: AspectHealth::Healthy,
            clipboard: AspectHealth::Healthy,
            session: AspectHealth::Healthy,
            overall: OverallStatus::Healthy,
        }
    }
}

impl SessionStatus {
    /// Apply one monitor event and recompute the aggregate state.
    ///
    /// Returns `(old_overall, new_overall)` so the monitor can decide whether
    /// to announce a state transition after broadcasting the update.
    pub(crate) fn apply_event(
        &mut self,
        event: SessionStatusEvent,
    ) -> (OverallStatus, OverallStatus) {
        let old_overall = self.overall;

        match event {
            SessionStatusEvent::AspectHealthChanged(change) => {
                self.apply_aspect_health(change);
            }
            SessionStatusEvent::SessionClosed { reason } => {
                error!("Session closed by compositor: {reason}");
                self.session = AspectHealth::Failed(reason);
                self.input = AspectHealth::Failed("session closed".into());
                self.clipboard = AspectHealth::Failed("session closed".into());
            }
            SessionStatusEvent::SessionInvalidated { reason } => {
                warn!("Session invalidated: {reason}");
                self.session = AspectHealth::Failed(reason);
                self.input = AspectHealth::Failed("session invalidated".into());
                self.clipboard = AspectHealth::Failed("session invalidated".into());
            }
            SessionStatusEvent::GraphicsStreamStateChanged { state } => {
                self.apply_graphics_stream_state(state);
            }
            SessionStatusEvent::VideoFrameStalled { stall_duration_ms } => {
                warn!("Graphics frames stalled for {stall_duration_ms}ms");
                self.graphics =
                    AspectHealth::Degraded(format!("no frames for {stall_duration_ms}ms"));
            }
            SessionStatusEvent::VideoFrameNeverStarted { elapsed_ms } => {
                error!("No graphics frames received since session start ({elapsed_ms}ms elapsed)");
                self.graphics = AspectHealth::Failed(format!(
                    "capture never delivered frames ({elapsed_ms}ms)"
                ));
            }
            SessionStatusEvent::VideoFrameResumed => {
                if !self.graphics.is_healthy() {
                    info!("Graphics frames resumed after stall");
                    self.graphics = AspectHealth::Healthy;
                }
            }
            SessionStatusEvent::InputFailed { reason, permanent } => {
                if permanent {
                    error!("Input permanently failed: {reason}");
                    self.input = AspectHealth::Failed(reason);
                } else {
                    warn!("Input transiently failed: {reason}");
                    self.input = AspectHealth::Degraded(reason);
                }
            }
            SessionStatusEvent::InputRecovered => {
                if !self.input.is_healthy() {
                    info!("Input recovered");
                    self.input = AspectHealth::Healthy;
                }
            }
            SessionStatusEvent::ClipboardFailed { reason } => {
                warn!("Clipboard failed: {reason}");
                self.clipboard = AspectHealth::Failed(reason);
            }
            SessionStatusEvent::ClipboardRecovered => {
                if !self.clipboard.is_healthy() {
                    info!("Clipboard recovered");
                    self.clipboard = AspectHealth::Healthy;
                }
            }
            SessionStatusEvent::CompositorLost { bus_name } => {
                error!("Compositor D-Bus name lost: {bus_name}");
                self.session = AspectHealth::Failed(format!("compositor lost: {bus_name}"));
                self.input = AspectHealth::Failed("compositor lost".into());
                self.graphics = AspectHealth::Degraded("compositor may have restarted".into());
            }
            SessionStatusEvent::EisStreamEnded { reason } => {
                warn!("EIS stream ended: {reason}");
                self.input = AspectHealth::Failed(reason);
            }
            SessionStatusEvent::SubsystemNotAvailable { subsystem } => {
                debug!("{subsystem} not available in this session");
                self.mark_subsystem_not_available(&subsystem);
            }
        }

        self.recompute_overall();
        (old_overall, self.overall)
    }

    fn apply_aspect_health(&mut self, change: AspectHealthChanged) {
        match change.aspect {
            SessionAspectId::Session => self.session = change.health,
            SessionAspectId::Graphics => self.graphics = change.health,
            SessionAspectId::Input => self.input = change.health,
            SessionAspectId::Clipboard => self.clipboard = change.health,
        }
    }

    fn apply_graphics_stream_state(&mut self, stream_state: GraphicsStreamState) {
        match stream_state {
            GraphicsStreamState::Streaming => {
                if !self.graphics.is_healthy() {
                    info!("Graphics stream recovered: streaming");
                }
                self.graphics = AspectHealth::Healthy;
            }
            GraphicsStreamState::Paused => {
                warn!("Graphics stream paused");
                self.graphics = AspectHealth::Degraded("PipeWire stream paused".into());
            }
            GraphicsStreamState::Error => {
                error!("Graphics stream error");
                self.graphics = AspectHealth::Failed("PipeWire stream error".into());
            }
        }
    }

    fn mark_subsystem_not_available(&mut self, subsystem: &str) {
        match subsystem {
            "clipboard" => self.clipboard = AspectHealth::NotApplicable,
            "input" => self.input = AspectHealth::NotApplicable,
            _ => {}
        }
    }

    /// Recompute `overall` from individual subsystem states
    fn recompute_overall(&mut self) {
        if self.session.is_failed() {
            // Session destruction is fatal — everything depends on it
            self.overall = OverallStatus::Invalid;
        } else if self.graphics.is_failed() || self.input.is_failed() {
            // Core subsystem failure
            self.overall = OverallStatus::Invalid;
        } else if !self.graphics.is_healthy()
            || !self.input.is_healthy()
            || !self.clipboard.is_healthy()
            || !self.session.is_healthy()
        {
            self.overall = OverallStatus::Degraded;
        } else {
            self.overall = OverallStatus::Healthy;
        }
    }
}

/// Overall status computed from aspect states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverallStatus {
    /// All subsystems healthy
    Healthy,
    /// Some subsystems degraded but session usable
    Degraded,
    /// Session invalid — recovery or teardown needed
    Invalid,
    /// Recovery in progress
    Recovering,
}

impl fmt::Display for OverallStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Invalid => write!(f, "invalid"),
            Self::Recovering => write!(f, "recovering"),
        }
    }
}

/// Events reported by runtime subsystems to the session supervisor.
#[derive(Debug, Clone)]
pub enum SessionStatusEvent {
    /// A session aspect reported a normalized health change.
    AspectHealthChanged(AspectHealthChanged),

    /// Managed compositor session was closed
    SessionClosed { reason: String },

    /// Managed compositor session reported invalid
    SessionInvalidated { reason: String },

    /// Graphics stream state changed.
    GraphicsStreamStateChanged { state: GraphicsStreamState },

    /// No graphics frames received for an extended period while stream is active
    VideoFrameStalled {
        /// How long since the last frame, in milliseconds
        stall_duration_ms: u64,
    },

    /// Graphics capture never produced any frames within the expected startup period.
    /// Unlike `VideoFrameStalled` (which fires after frames stop), this fires
    /// when frames never started -- e.g., ext-capture handshake succeeded but
    /// no pixel data was ever delivered.
    VideoFrameNeverStarted {
        /// How long we waited before declaring failure, in milliseconds
        elapsed_ms: u64,
    },

    /// Video frame timing recovered after a stall
    VideoFrameResumed,

    /// Input injection failed with a transient or permanent error
    InputFailed { reason: String, permanent: bool },

    /// Input recovered after a failure
    InputRecovered,

    /// Clipboard provider health check failed
    ClipboardFailed { reason: String },

    /// Clipboard provider recovered
    ClipboardRecovered,

    /// Compositor D-Bus name disappeared (restart or crash).
    CompositorLost { bus_name: String },

    /// EIS event stream ended (device loss)
    EisStreamEnded { reason: String },

    /// A subsystem is not available in this session configuration
    SubsystemNotAvailable { subsystem: String },
}

/// Graphics stream states relevant to session supervision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsStreamState {
    Streaming,
    Paused,
    Error,
}

impl fmt::Display for GraphicsStreamState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Streaming => write!(f, "streaming"),
            Self::Paused => write!(f, "paused"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Handle for subscribing to session status changes.
///
/// Subsystems hold a `SessionStatusSubscriber` to check current status or await changes.
#[derive(Clone)]
pub struct SessionStatusSubscriber {
    rx: tokio::sync::watch::Receiver<SessionStatus>,
}

impl SessionStatusSubscriber {
    /// Current session status (non-blocking)
    pub fn current(&self) -> SessionStatus {
        self.rx.borrow().clone()
    }

    /// Current overall health (non-blocking)
    pub fn overall(&self) -> OverallStatus {
        self.rx.borrow().overall
    }

    /// Whether the session is still valid.
    pub fn is_session_valid(&self) -> bool {
        !matches!(self.overall(), OverallStatus::Invalid)
    }

    /// Wait for the next session status change
    pub async fn changed(&mut self) -> Result<(), tokio::sync::watch::error::RecvError> {
        self.rx.changed().await
    }
}

/// Handle for reporting session status events from subsystems.
#[derive(Clone)]
pub struct SessionStatusReporter {
    tx: tokio::sync::mpsc::UnboundedSender<SessionStatusEvent>,
}

impl SessionStatusReporter {
    /// Report a status event to the supervisor.
    pub fn report(&self, event: SessionStatusEvent) {
        // Best-effort: if the supervisor is gone, the event is dropped.
        let _ = self.tx.send(event);
    }

    /// Report a normalized aspect health change.
    pub fn report_aspect_health(&self, aspect: SessionAspectId, health: AspectHealth) {
        self.report(SessionStatusEvent::AspectHealthChanged(
            AspectHealthChanged { aspect, health },
        ));
    }
}

/// Reporter alias for aspect-owned status producers.
pub type AspectStatusReporter = SessionStatusReporter;

/// Build a human-readable detail string from the session status,
/// focusing on unhealthy subsystems.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_health_state() {
        let state = SessionStatus::default();
        assert_eq!(state.overall, OverallStatus::Healthy);
        assert!(state.graphics.is_healthy());
        assert!(state.input.is_healthy());
        assert!(state.clipboard.is_healthy());
        assert!(state.session.is_healthy());
    }

    #[test]
    fn test_recompute_session_failed_makes_invalid() {
        let mut state = SessionStatus::default();
        state.session = AspectHealth::Failed("closed".into());
        state.recompute_overall();
        assert_eq!(state.overall, OverallStatus::Invalid);
    }

    #[test]
    fn test_recompute_degraded() {
        let mut state = SessionStatus::default();
        state.clipboard = AspectHealth::Degraded("slow".into());
        state.recompute_overall();
        assert_eq!(state.overall, OverallStatus::Degraded);
    }

    #[test]
    fn test_recompute_graphics_failed_makes_invalid() {
        let mut state = SessionStatus::default();
        state.graphics = AspectHealth::Failed("stream error".into());
        state.recompute_overall();
        assert_eq!(state.overall, OverallStatus::Invalid);
    }

    #[test]
    fn test_aspect_health_changed_updates_graphics() {
        let mut state = SessionStatus::default();
        state.apply_event(SessionStatusEvent::AspectHealthChanged(
            AspectHealthChanged {
                aspect: SessionAspectId::Graphics,
                health: AspectHealth::Failed("stream error".into()),
            },
        ));
        assert_eq!(state.overall, OverallStatus::Invalid);
        assert!(state.graphics.is_failed());
    }

    #[test]
    fn test_subsystem_health_display() {
        assert_eq!(AspectHealth::Healthy.to_string(), "healthy");
        assert_eq!(
            AspectHealth::Failed("gone".into()).to_string(),
            "failed: gone"
        );
    }
}
