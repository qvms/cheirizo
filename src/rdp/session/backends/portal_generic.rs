//! Embedded portal-generic backend: native capture + input + clipboard.
//!
//! Uses the in-process portal-generic backend to provide screen capture,
//! input injection, and clipboard support for the managed compositor runtime
//! without requiring a separate portal daemon process.
//!
//! # Protocols Used
//!
//! - **Capture**: ext-image-copy-capture-v1 or wlr-screencopy-v1
//! - **Input**: wlr-virtual-pointer + zwp-virtual-keyboard (or EIS bridge)
//! - **Clipboard**: ext-data-control-v1 or wlr-data-control-v1
//!
//! # Architecture
//!
//! ```text
//! PortalSessionBackend
//!   ├─> WaylandConnection (global registry scan)
//!   ├─> direct frame channel from screencopy capture
//!   └─> PortalGenericSessionHandle
//!       ├─> CaptureBackend → compositor frames
//!       ├─> InputBackend → virtual keyboard/pointer injection
//!       └─> ClipboardBackend → data-control read/write
//! ```
//!
//! # Runtime characteristics
//!
//! - Provides video capture, input injection, and clipboard support in one backend
//! - Keeps capture/input/clipboard wiring local to the managed compositor session
//!
//! # Limitations
//!
//! - Not Flatpak-compatible (requires direct Wayland socket access)
//! - PipeWire is not required for the managed compositor's direct-frame capture path

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::desktop::portal::xdg_desktop::{
    CaptureProtocol, InputBackend, InputEvent, KeyState, KeyboardEvent, PointerEvent,
    pipewire::PipeWireManager,
    services::{
        capture::create_capture_backend, clipboard::create_clipboard_backend,
        input::create_input_backend,
    },
    types::{CursorMode, DeviceTypes, SourceType},
    wayland::WaylandConnection,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::{debug, info, warn};

use crate::{
    rdp::session::backend::{DirectFrameReceiver, SessionHandle, StreamInfo},
    rdp::session::supervision::{SessionStatusEvent, SessionStatusReporter},
};

/// Session backend using embedded portal-generic services.
///
/// Connects directly to the Wayland compositor as a client and provides
/// video capture, input injection, and clipboard via native protocols.
pub struct PortalSessionBackend;

/// Stops a newly spawned Wayland loop if session construction fails before
/// ownership can be transferred to the session handle.
struct WaylandStopGuard {
    stop: Arc<AtomicBool>,
    armed: bool,
}

impl WaylandStopGuard {
    fn new(stop: Arc<AtomicBool>) -> Self {
        Self { stop, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for WaylandStopGuard {
    fn drop(&mut self) {
        if self.armed {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn prepare_wayland<F>(
    connect: F,
    capture: &crate::desktop::portal::xdg_desktop::services::capture::CapturePreference,
) -> Result<(
    WaylandConnection,
    crate::desktop::portal::xdg_desktop::wayland::AvailableProtocols,
    Vec<crate::desktop::portal::xdg_desktop::types::SourceInfo>,
)>
where
    F: FnOnce() -> crate::desktop::portal::xdg_desktop::Result<WaylandConnection>,
{
    let mut connection = connect().context("connect to managed Wayland display")?;
    if capture.handshake_timeout_ms != 0 {
        connection.set_ext_capture_handshake_timeout(std::time::Duration::from_millis(
            capture.handshake_timeout_ms,
        ));
    }
    let avoid_ext = capture.preferred == Some(CaptureProtocol::WlrScreencopy)
        || capture
            .broken_protocols
            .contains(&CaptureProtocol::ExtImageCopyCapture);
    connection.set_force_wlr_screencopy(avoid_ext);
    let protocols = connection.available_protocols().clone();
    let sources = connection.state().get_sources();
    Ok((connection, protocols, sources))
}

fn attach_input(
    mut settings: crate::desktop::portal::xdg_desktop::services::input::InputBackendConfig,
    protocols: &crate::desktop::portal::xdg_desktop::wayland::AvailableProtocols,
    socket: Option<PathBuf>,
) -> Result<(String, Box<dyn InputBackend>)> {
    settings.wlr.wayland_socket_path = socket;
    let mut backend = create_input_backend(&settings, protocols)
        .map_err(|error| anyhow::anyhow!("create input backend: {error}"))?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    backend
        .create_context(
            &id,
            DeviceTypes {
                keyboard: true,
                pointer: true,
                touchscreen: false,
            },
        )
        .map_err(|error| anyhow::anyhow!("create input context: {error}"))?;
    Ok((id, backend))
}

fn start_monitor_capture(
    settings: crate::desktop::portal::xdg_desktop::services::capture::CapturePreference,
    protocols: &crate::desktop::portal::xdg_desktop::wayland::AvailableProtocols,
    sources: Vec<crate::desktop::portal::xdg_desktop::types::SourceInfo>,
    pipewire: Arc<PipeWireManager>,
    commands: std::sync::mpsc::Sender<crate::desktop::portal::xdg_desktop::wayland::CaptureCommand>,
) -> Result<(
    Box<dyn crate::desktop::portal::xdg_desktop::CaptureBackend>,
    Vec<StreamInfo>,
)> {
    let mut backend = create_capture_backend(protocols, &settings, sources, pipewire, commands)
        .map_err(|error| anyhow::anyhow!("create capture backend: {error}"))?;
    let monitors = backend
        .get_sources(&[SourceType::Monitor])
        .map_err(|error| anyhow::anyhow!("enumerate capture monitors: {error}"))?;
    let streams = backend
        .create_capture_session(&monitors, CursorMode::Hidden)
        .map_err(|error| anyhow::anyhow!("start monitor capture: {error}"))?
        .into_iter()
        .map(|stream| {
            StreamInfo::new(
                stream.node_id,
                stream.size.0,
                stream.size.1,
                stream.position.0,
                stream.position.1,
            )
        })
        .collect();
    Ok((backend, streams))
}

impl PortalSessionBackend {
    pub async fn create_session_for_wayland_socket(
        path: PathBuf,
        settings: crate::config::PortalStartupSettings,
    ) -> Result<Arc<dyn SessionHandle>> {
        let explicit_wayland_socket = path.clone();
        Self::create_session_with_wayland(
            move || WaylandConnection::connect_to_path(&path),
            Some(explicit_wayland_socket),
            settings,
        )
        .await
    }

    async fn create_session_with_wayland<F>(
        connect_wayland: F,
        explicit_wayland_socket: Option<PathBuf>,
        settings: crate::config::PortalStartupSettings,
    ) -> Result<Arc<dyn SessionHandle>>
    where
        F: FnOnce() -> crate::desktop::portal::xdg_desktop::Result<WaylandConnection>
            + Send
            + 'static,
    {
        info!("portal-generic: Creating session with embedded portal backend");

        // All Wayland and PipeWire setup is synchronous; run on blocking thread
        let handle = tokio::task::spawn_blocking(move || -> Result<_> {
            let (wayland, protocols, sources) =
                prepare_wayland(connect_wayland, &settings.capture)?;
            let pipewire = Arc::new(PipeWireManager::disabled());
            let (raw_frame_tx, raw_frame_rx) = std::sync::mpsc::channel();
            let (stop, _, capture_tx, clipboard_tx, shared_clipboard, _) = wayland
                .spawn_event_loop_with_frame_channel(Arc::clone(&pipewire), Some(raw_frame_tx));
            let mut rollback = WaylandStopGuard::new(Arc::clone(&stop));

            let (session_id, input_backend) =
                attach_input(settings.input, &protocols, explicit_wayland_socket)?;
            let (capture_backend, streams) = start_monitor_capture(
                settings.capture,
                &protocols,
                sources,
                Arc::clone(&pipewire),
                capture_tx,
            )?;
            let clipboard_backend = create_clipboard_backend(
                &protocols,
                &settings.clipboard,
                clipboard_tx,
                shared_clipboard,
            )
            .map(|backend| Arc::new(Mutex::new(backend)));

            rollback.disarm();
            Ok(PortalGenericSessionHandle {
                session_id,
                first_pointer_event: std::sync::atomic::AtomicBool::new(false),
                input_backend: Arc::new(Mutex::new(input_backend)),
                _capture_backend: Arc::new(Mutex::new(capture_backend)),
                clipboard_backend,
                _pipewire_manager: pipewire,
                streams: std::sync::Mutex::new(streams),
                frame_rx: std::sync::Mutex::new(Some(raw_frame_rx)),
                wayland_stop: stop,
                health_reporter: std::sync::OnceLock::new(),
            })
        })
        .await
        .context("portal-generic: Setup task panicked")??;

        Ok(Arc::new(handle))
    }
}

/// Session handle for the embedded portal-generic backend.
///
/// Bridges portal-generic backend traits to the `SessionHandle` interface
/// consumed by the RDP server session layer.
struct PortalGenericSessionHandle {
    session_id: String,
    first_pointer_event: std::sync::atomic::AtomicBool,
    input_backend: Arc<Mutex<Box<dyn InputBackend>>>,
    _capture_backend: Arc<Mutex<Box<dyn crate::desktop::portal::xdg_desktop::CaptureBackend>>>,
    clipboard_backend:
        Option<Arc<Mutex<Box<dyn crate::desktop::portal::xdg_desktop::ClipboardBackend>>>>,
    _pipewire_manager: Arc<PipeWireManager>,
    streams: std::sync::Mutex<Vec<StreamInfo>>,
    /// Direct frame channel receiver (taken once by the display handler).
    frame_rx: std::sync::Mutex<
        Option<std::sync::mpsc::Receiver<crate::desktop::portal::xdg_desktop::RawFrame>>,
    >,
    wayland_stop: Arc<AtomicBool>,
    health_reporter: std::sync::OnceLock<SessionStatusReporter>,
}

impl Drop for PortalGenericSessionHandle {
    fn drop(&mut self) {
        self.wayland_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
        debug!("portal-generic: Wayland event loop stop signaled");
        if let Some(reporter) = self.health_reporter.get() {
            reporter.report(SessionStatusEvent::SessionClosed {
                reason: "portal-generic session dropped".into(),
            });
        }
    }
}

/// Get current time in microseconds for event timestamps.
fn current_time_usec() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn normalize_absolute_pointer_position(
    streams: &[StreamInfo],
    stream_id: u32,
    x: f64,
    y: f64,
) -> (f64, f64) {
    if x <= 1.0 && y <= 1.0 {
        return (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    }

    let stream = streams
        .iter()
        .find(|stream| stream.node_id == stream_id)
        .or_else(|| streams.first());

    let Some(stream) = stream else {
        return (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    };

    let width = f64::from(stream.width.max(1));
    let height = f64::from(stream.height.max(1));
    ((x / width).clamp(0.0, 1.0), (y / height).clamp(0.0, 1.0))
}

impl PortalGenericSessionHandle {
    fn inject(&self, event: InputEvent, operation: &str) -> Result<()> {
        self.input_backend
            .lock()
            .map_err(|_| anyhow::anyhow!("input backend lock poisoned"))?
            .inject_event(&self.session_id, event)
            .map_err(|error| anyhow::anyhow!("{operation}: {error}"))
    }
}

#[async_trait]
impl SessionHandle for PortalGenericSessionHandle {
    fn set_health_reporter(&self, reporter: SessionStatusReporter) {
        let _ = self.health_reporter.set(reporter);
    }

    fn direct_frame_receiver(&self) -> Result<DirectFrameReceiver> {
        // Use direct frame channel — PipeWire buffer sharing doesn't work
        // across separate connections (the buffer data pointer is NULL on the
        // consumer side because the source's ALLOC_BUFFERS creates MemPtr
        // buffers that can't be shared across address spaces).
        let raw_rx = self
            .frame_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();

        let Some(raw_rx) = raw_rx else {
            anyhow::bail!("portal-generic direct frame channel was already taken");
        };

        info!("portal-generic: Using direct frame channel (bypassing PipeWire)");

        // Bridge RawFrame (portal crate) -> RawFrameData (pipewire crate).
        // Keep one output frame plus one replaceable pending frame. A plain
        // bounded channel's `try_send` would discard each *new* frame while
        // retaining stale queued frames, which makes latency grow during a
        // stall. Here newer capture replaces the pending frame instead.
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        if let Err(e) = std::thread::Builder::new()
            .name("raw-frame-bridge".into())
            .spawn(move || {
                let mut pending = None;
                let mut dropped: u64 = 0;
                loop {
                    if let Some(frame) = pending.take() {
                        match tx.try_send(frame) {
                            Ok(()) => {}
                            Err(std::sync::mpsc::TrySendError::Full(frame)) => {
                                pending = Some(frame);
                            }
                            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => break,
                        }
                    }

                    match raw_rx.recv_timeout(std::time::Duration::from_millis(8)) {
                        Ok(raw) => {
                            let converted = crate::desktop::pipewire::frame::RawFrameData {
                                data: raw.data,
                                width: Some(raw.width),
                                height: Some(raw.height),
                                stride: Some(raw.stride),
                                format: None,
                            };
                            if pending.replace(converted).is_some() {
                                dropped = dropped.saturating_add(1);
                                if dropped == 1 || dropped.is_multiple_of(300) {
                                    warn!(
                                        "portal-generic: replaced {dropped} stale direct frames to keep latency low"
                                    );
                                }
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            if let Some(frame) = pending {
                                let _ = tx.send(frame);
                            }
                            break;
                        }
                    }
                }
                info!("portal-generic: raw-frame-bridge thread exited");
            })
        {
            anyhow::bail!("failed to spawn raw-frame-bridge thread: {e}");
        }

        Ok(rx)
    }

    fn streams(&self) -> Vec<StreamInfo> {
        self.streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set_streams(&self, streams: Vec<StreamInfo>) {
        *self
            .streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = streams;
    }

    async fn notify_keyboard_keycode(&self, keycode: i32, pressed: bool) -> Result<()> {
        let event = InputEvent::Keyboard(KeyboardEvent {
            keycode: keycode as u32,
            state: if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            },
            time_usec: current_time_usec(),
        });

        self.inject(event, "keyboard injection")
    }

    async fn notify_pointer_motion_absolute(&self, stream_id: u32, x: f64, y: f64) -> Result<()> {
        // The embedded portal path follows the RemoteDesktop portal contract:
        // absolute pointer coordinates are normalized 0.0–1.0 within the selected
        // PipeWire stream. wrdp's RDP coordinate transformer produces pixel
        // coordinates in stream space, so normalize them here before injecting
        // into the embedded backend. Without this, wlr_virtual_pointer receives
        // huge absolute values and the pointer appears inert/off-screen.
        let streams = self
            .streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (x, y) = normalize_absolute_pointer_position(&streams, stream_id, x, y);
        if !self
            .first_pointer_event
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            info!(
                stream_id,
                normalized_x = x,
                normalized_y = y,
                "First normalized pointer event"
            );
        }

        let event = InputEvent::Pointer(PointerEvent::MotionAbsolute {
            x,
            y,
            stream: stream_id,
            time_usec: current_time_usec(),
        });

        self.inject(event, "absolute pointer injection")
    }

    async fn notify_pointer_button(&self, button: i32, pressed: bool) -> Result<()> {
        let event = InputEvent::Pointer(PointerEvent::Button {
            button: button as u32,
            state: if pressed {
                crate::desktop::portal::xdg_desktop::ButtonState::Pressed
            } else {
                crate::desktop::portal::xdg_desktop::ButtonState::Released
            },
            time_usec: current_time_usec(),
        });

        self.inject(event, "pointer button injection")
    }

    async fn notify_pointer_axis(&self, dx: f64, dy: f64) -> Result<()> {
        let event = InputEvent::Pointer(PointerEvent::Scroll {
            dx,
            dy,
            time_usec: current_time_usec(),
        });

        self.inject(event, "pointer scroll injection")
    }

    fn clipboard_source(&self) -> crate::rdp::session::backend::ClipboardSource {
        match self.clipboard_backend.as_ref() {
            Some(backend) => {
                crate::rdp::session::backend::ClipboardSource::DataControl(Arc::clone(backend))
            }
            None => crate::rdp::session::backend::ClipboardSource::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_time_usec() {
        let time = current_time_usec();
        assert!(time > 0);
    }

    #[test]
    fn test_normalize_absolute_pointer_position_from_pixels() {
        let streams = vec![StreamInfo::new(7, 1920, 1080, 0, 0)];

        let (x, y) = normalize_absolute_pointer_position(&streams, 7, 960.0, 540.0);

        assert!((x - 0.5).abs() < f64::EPSILON);
        assert!((y - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_normalize_absolute_pointer_position_clamps_without_streams() {
        let (x, y) = normalize_absolute_pointer_position(&[], 7, 2.0, -1.0);

        assert_eq!((x, y), (1.0, 0.0));
    }
}
