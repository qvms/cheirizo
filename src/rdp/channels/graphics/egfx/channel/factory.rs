//! EGFX channel factory for IronRDP server integration.
//!
//! Implements `ironrdp_server::GfxServerFactory` and wires shared EGFX server
//! state so channel attach, capability negotiation, and frame dispatch can
//! coordinate through the runtime display pipeline.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer};
use ironrdp_graphics::zgfx::CompressionMode;
use ironrdp_server::{
    GfxDvcBridge, GfxServerFactory, GfxServerHandle, ServerEvent, ServerEventSender,
};
use tokio::sync::{RwLock, mpsc};

use crate::rdp::channels::graphics::egfx::WrdpGraphicsHandler;

/// Factory for creating EGFX graphics-pipeline handlers.
///
/// Passed to the RDP server builder to create a shared
/// `GraphicsPipelineServer` per client connection.
///
/// # Platform Quirks
///
/// The factory accepts a `force_avc420_only` flag which is passed to the handler.
/// This is used when platform detection (e.g., RHEL 9) identifies that AVC444
/// produces visual artifacts. The handler will then disable AVC444 regardless
/// of client capability.
///
/// # Usage
///
/// ```ignore
/// // Check if platform has AVC444 quirk
/// let force_avc420 = capabilities.profile.has_quirk(&Quirk::ForceAvc420);
///
/// let gfx_factory = EgfxChannelFactory::with_quirks(width, height, force_avc420);
///
/// // Get handle for display handler before passing to RdpServer
/// let gfx_handle = gfx_factory.server_handle();
///
/// let server = RdpServer::builder()
///     .with_gfx_handler(gfx_factory)
///     // ...
///     .build();
///
/// // Display handler uses gfx_handle to send frames
/// display_handler.set_gfx_server(gfx_handle);
/// ```
pub struct EgfxChannelFactory {
    /// Initial desktop dimensions
    width: u16,
    height: u16,

    /// Shared state for checking handler readiness from other parts of the server
    handler_state: Arc<RwLock<Option<HandlerState>>>,

    /// Shared GraphicsPipelineServer for proactive frame sending
    /// Created lazily on first call to build_server_with_handle()
    server_handle: Arc<RwLock<Option<GfxServerHandle>>>,

    /// Server codec policy intersected with client wire capabilities.
    codec_policy: EgfxCodecPolicy,

    /// Maximum frames in flight before backpressure
    max_frames_in_flight: u32,

    /// ZGFX compression mode for EGFX data
    compression_mode: CompressionMode,
}

/// Server policy intersected with the client's RDPGFX capability flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgfxCodecPolicy {
    Auto,
    Avc420,
    Avc444,
    Bitmap,
}

impl EgfxCodecPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "avc420" => Some(Self::Avc420),
            "avc444" => Some(Self::Avc444),
            "bitmap" => Some(Self::Bitmap),
            _ => None,
        }
    }
}

/// Explicit output path selected for a connected client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegotiatedEgfxMode {
    Bitmap,
    Avc420,
    Avc444,
}

impl NegotiatedEgfxMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Bitmap => "FastPath bitmap",
            Self::Avc420 => "AVC420",
            Self::Avc444 => "AVC444",
        }
    }

    pub fn uses_avc(self) -> bool {
        matches!(self, Self::Avc420 | Self::Avc444)
    }
}

/// Shared handler state accessible from the display pipeline.
///
/// Updated by `WrdpGraphicsHandler` callbacks and read by `EgfxChannelSender`
/// to determine EGFX readiness and channel metadata.
#[derive(Debug, Clone, Default)]
pub struct HandlerState {
    /// Whether EGFX channel is ready (capabilities negotiated)
    pub is_ready: bool,
    /// Explicit negotiated EGFX mode for this client.
    pub negotiated_mode: Option<NegotiatedEgfxMode>,
    /// Whether AVC420 (H.264 YUV420) codec is supported
    pub is_avc420_enabled: bool,
    /// Whether AVC444 (H.264 YUV444) codec is supported
    pub is_avc444_enabled: bool,
    /// Whether this client needs Android RD Client pointer workaround updates.
    ///
    /// Android clients that negotiate EGFX with AVC-disabled/Planar do not reliably draw
    /// a visible local pointer unless the server sends explicit pointer PDUs.
    /// Windows clients must not receive this workaround because the Android cursor
    /// bitmap is vertically flipped for that client quirk.
    pub needs_android_pointer_updates: bool,
    /// Primary surface ID for frame sending (None = no surface yet)
    /// Note: Surface ID 0 is valid in EGFX, so we use Option
    pub primary_surface_id: Option<u16>,
    /// DVC channel ID assigned to EGFX (needed for encode_dvc_messages)
    pub dvc_channel_id: u32,
}

/// Type alias for shared handler state
pub type SharedHandlerState = Arc<RwLock<Option<HandlerState>>>;

impl EgfxChannelFactory {
    pub fn new(width: u16, height: u16) -> Self {
        Self::with_config(width, height, false, 3, CompressionMode::Never)
    }

    /// Use this constructor when platform detection has identified quirks
    /// that affect codec selection (e.g., RHEL 9 AVC444 blur issue).
    pub fn with_quirks(width: u16, height: u16, force_avc420_only: bool) -> Self {
        Self::with_config(width, height, force_avc420_only, 3, CompressionMode::Never)
    }

    pub fn with_config(
        width: u16,
        height: u16,
        force_avc420_only: bool,
        max_frames_in_flight: u32,
        compression_mode: CompressionMode,
    ) -> Self {
        Self {
            width,
            height,
            handler_state: Arc::new(RwLock::new(None)),
            server_handle: Arc::new(RwLock::new(None)),
            codec_policy: if force_avc420_only {
                EgfxCodecPolicy::Avc420
            } else {
                EgfxCodecPolicy::Auto
            },
            max_frames_in_flight,
            compression_mode,
        }
    }

    pub fn with_codec_policy(mut self, policy: EgfxCodecPolicy) -> Self {
        self.codec_policy = policy;
        self
    }

    #[cfg(test)]
    fn codec_policy(&self) -> EgfxCodecPolicy {
        self.codec_policy
    }

    /// Get shared reference to handler state
    ///
    /// This can be used by the display handler to check if EGFX is ready
    /// and which codecs are available.
    pub fn handler_state(&self) -> Arc<RwLock<Option<HandlerState>>> {
        Arc::clone(&self.handler_state)
    }

    /// Get the shared GraphicsPipelineServer handle
    ///
    /// This returns the handle that was created by `build_server_with_handle()`.
    /// Use this to access the server for frame sending from the display handler.
    ///
    /// Returns `None` if `build_server_with_handle()` hasn't been called yet
    /// (i.e., the RDP connection hasn't started the channel attachment phase).
    pub fn server_handle(&self) -> Arc<RwLock<Option<GfxServerHandle>>> {
        Arc::clone(&self.server_handle)
    }
}

impl ServerEventSender for EgfxChannelFactory {
    fn set_sender(&mut self, _sender: mpsc::UnboundedSender<ServerEvent>) {
        // GFX factory doesn't need the server event sender directly;
        // EgfxChannelSender already has its own event_tx from server setup.
    }
}

fn retry_rwlock_write<T>(lock: &RwLock<T>, mut update: impl FnMut(&mut T)) -> bool {
    const MAX_ATTEMPTS: usize = 100;
    const YIELD_ATTEMPTS: usize = 10;

    for attempt in 0..MAX_ATTEMPTS {
        if let Ok(mut guard) = lock.try_write() {
            update(&mut guard);
            return true;
        }

        if attempt < YIELD_ATTEMPTS {
            std::thread::yield_now();
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    false
}

impl GfxServerFactory for EgfxChannelFactory {
    fn build_gfx_handler(&self) -> Box<dyn GraphicsPipelineHandler> {
        let handler = WrdpGraphicsHandler::with_config(
            self.width,
            self.height,
            Arc::new(RwLock::new(None)),
            self.codec_policy,
            self.max_frames_in_flight,
        );
        Box::new(handler)
    }

    fn build_server_with_handle(&self) -> Option<(GfxDvcBridge, GfxServerHandle)> {
        tracing::info!(
            width = self.width,
            height = self.height,
            "EGFX: attaching fresh rdpgfx DVC server for connection"
        );
        // This is called while IronRDP attaches channels for a new connection.
        // Clear readiness here, before the new client's EGFX capability exchange;
        // the handler below will repopulate it from on_ready(). Do not clear this
        // later from the display pipeline, because that races with Android's fast
        // AVC-disabled/Planar negotiation and leaves the pipeline stuck in bitmap fallback.
        retry_rwlock_write(&self.handler_state, |state| *state = None);

        // Handler updates handler_state when callbacks are invoked,
        // allowing EgfxChannelSender to check EGFX readiness
        let handler = WrdpGraphicsHandler::with_config(
            self.width,
            self.height,
            Arc::clone(&self.handler_state),
            self.codec_policy,
            self.max_frames_in_flight,
        );

        // std::sync::Mutex (not tokio) because DvcProcessor trait
        // has synchronous methods that cannot use async locks
        let server = Arc::new(Mutex::new(GraphicsPipelineServer::with_compression(
            Box::new(handler),
            self.compression_mode,
        )));

        // This callback is synchronous, while the display pipeline polls the
        // same tokio RwLock from async code. A single try_write() can lose the
        // new per-connection handle under read contention, leaving EGFX
        // negotiated but permanently "not ready" until bitmap fallback crashes
        // Android with 0xd06/0x200d. Retry briefly: readers hold the lock only
        // for a very short readiness check.
        if !retry_rwlock_write(&self.server_handle, |handle| {
            *handle = Some(Arc::clone(&server));
        }) {
            tracing::error!("EGFX: failed to store GfxServerHandle after retries");
        }

        let bridge = GfxDvcBridge::new(Arc::clone(&server));

        Some((bridge, server))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_policy_parser_rejects_unknown_values() {
        assert_eq!(EgfxCodecPolicy::parse("auto"), Some(EgfxCodecPolicy::Auto));
        assert_eq!(
            EgfxCodecPolicy::parse("avc420"),
            Some(EgfxCodecPolicy::Avc420)
        );
        assert_eq!(
            EgfxCodecPolicy::parse("avc444"),
            Some(EgfxCodecPolicy::Avc444)
        );
        assert_eq!(
            EgfxCodecPolicy::parse("bitmap"),
            Some(EgfxCodecPolicy::Bitmap)
        );
        assert_eq!(EgfxCodecPolicy::parse("planar"), None);
    }

    #[test]
    fn factory_carries_explicit_bitmap_policy() {
        let factory = EgfxChannelFactory::new(1280, 720).with_codec_policy(EgfxCodecPolicy::Bitmap);
        assert_eq!(factory.codec_policy(), EgfxCodecPolicy::Bitmap);
    }
}
