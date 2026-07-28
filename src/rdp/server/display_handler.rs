//! Display pipeline handler for wrdp RDP connections.
//!
//! Implements IronRDP display traits and drives the runtime frame path from
//! managed compositor capture into RDP display updates.
//!
//! # Overview
//!
//! Handles frame intake, conversion/encoding decisions, and update delivery for
//! the active authenticated client.
//!
//! # Architecture
//!
//! ```text
//! Wayland Compositor
//!        │
//!        ├─> Portal ScreenCast API
//!        │
//!        ▼
//! PipeWire Streams (one per monitor)
//!        │
//!        ├─> PipeWireThreadManager
//!        │     └─> Frame extraction via process() callback
//!        │
//!        ▼
//! Frame Channel (std::sync::mpsc)
//!        │
//!        ├─> Display Handler (async task)
//!        │     ├─> BitmapConverter (VideoFrame → RDP bitmap)
//!        │     └─> Format mapping (BGRA/RGB → IronRDP formats)
//!        │
//!        ▼
//! DisplayUpdate Channel (tokio::mpsc)
//!        │
//!        ├─> IronRDP Server
//!        │     └─> RemoteFX encoding
//!        │
//!        ▼
//! RDP Client Display
//! ```
//!
//! # Frame Processing Pipeline
//!
//! 1. **Capture:** PipeWire thread extracts frame from buffer
//! 2. **Transfer:** Frame sent via channel (zero-copy Arc)
//! 3. **Convert:** BitmapConverter transforms to RDP format
//! 4. **Map:** Pixel formats mapped to IronRDP types
//! 5. **Stream:** DisplayUpdate sent to IronRDP
//! 6. **Encode:** IronRDP applies RemoteFX compression
//! 7. **Transmit:** Sent to RDP client over TLS
//!
//! # Pixel Format Handling
//!
//! The handler supports multiple pixel formats with intelligent conversion:
//!
//! - **BgrX32** → IronRDP::BgrX32 (direct mapping)
//! - **Bgr24** → IronRDP::XBgr32 (upsample to 32-bit)
//! - **Rgb16** → IronRDP::XRgb32 (upsample to 32-bit)
//! - **Rgb15** → IronRDP::XRgb32 (upsample to 32-bit)
//!
//! # Performance Characteristics
//!
//! - **Frame latency:** <3ms (PipeWire → IronRDP)
//! - **Channel capacity:** 64 frames buffered
//! - **Frame rate:** Non-blocking, supports up to 144Hz
//! - **Memory:** Zero-copy where possible (Arc<Vec<u8>>)

use std::{
    borrow::Cow,
    num::{NonZeroU16, NonZeroUsize},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result};
use bytes::Bytes;
use ironrdp_server::{
    BitmapUpdate as IronBitmapUpdate, DesktopSize, DisplayUpdate, GfxServerHandle,
    PixelFormat as IronPixelFormat, RdpServerDisplay, RdpServerDisplayUpdates, ServerEvent,
};
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, error, info, trace, warn};

use crate::{
    pipewire::{PipeWireThreadManager, VideoFrame},
    portal::StreamInfo,
    rdp::channels::graphics::bitmap::converter::{
        BitmapConverter, BitmapData, BitmapUpdate, RdpPixelFormat, Rectangle,
    },
    rdp::channels::graphics::damage::{DamageConfig, DamageDetector, DamageRegion},
    rdp::channels::graphics::egfx::channel::{EgfxChannelSender, HandlerState, NegotiatedEgfxMode},
    rdp::channels::graphics::egfx::{
        Avc420Encoder, Avc444Encoder, ColorSpaceConfig, EncoderConfig, align_to_16,
    },
    rdp::channels::graphics::performance::{
        AdaptiveFpsController, EncodingDecision, LatencyGovernor, LatencyMode,
    },
    rdp::server::{event_multiplexer::GraphicsFrame, input_handler::InputChannelHandler},
    services::RuntimeCapabilities,
};

#[cfg(feature = "vaapi")]
use crate::rdp::channels::graphics::egfx::{HardwareEncoder, create_hardware_encoder};

static LOGGED_FIRST_BITMAP_UPDATE: AtomicBool = AtomicBool::new(false);

/// Client-initiated resize request
///
/// Sent from `request_layout()` (sync context) to the pipeline loop (async)
/// via a bounded sync channel. The pipeline coalesces multiple requests
/// and executes the resize sequence.
struct ResizeRequest {
    width: u16,
    height: u16,
}

/// Video encoder abstraction for codec-agnostic frame encoding
///
/// Supports both AVC420 (standard H.264 4:2:0) and AVC444 (premium H.264 4:4:4).
/// The codec is selected at runtime based on client capability negotiation.
///
/// When `vaapi` feature is enabled and hardware encoding is enabled
/// in config, the `Hardware` variant wraps a GPU-accelerated encoder.
enum VideoEncoder {
    /// Standard H.264 with 4:2:0 chroma subsampling
    Avc420(Avc420Encoder),
    /// Premium H.264 with 4:4:4 chroma via dual-stream encoding
    Avc444(Avc444Encoder),
    /// GPU-accelerated H.264 via VA-API (AVC420 only)
    #[cfg(feature = "vaapi")]
    Hardware(SendHardwareEncoder),
}

/// Wrapper to make `Box<dyn HardwareEncoder>` `Send` for use in async tasks.
///
/// # Safety
/// The VA-API encoder uses `Rc<Display>` internally (not `Send`), but the encoder
/// is always created and used from the same async task — never shared across threads.
/// This wrapper asserts `Send` to satisfy the tokio requirement without actual
/// cross-thread access.
#[cfg(feature = "vaapi")]
struct SendHardwareEncoder(Box<dyn HardwareEncoder>);

#[cfg(feature = "vaapi")]
unsafe impl Send for SendHardwareEncoder {}

/// Result of encoding a frame - varies by codec
enum EncodedVideoFrame {
    /// Single H.264 stream (AVC420)
    Single(Vec<u8>),
    /// Dual H.264 streams (AVC444: main + auxiliary)
    /// Phase 1: aux is now Option for bandwidth optimization
    Dual {
        main: Vec<u8>,
        aux: Option<Vec<u8>>, // Optional for aux omission
    },
}

impl EncodedVideoFrame {
    fn payload_len(&self) -> usize {
        match self {
            Self::Single(data) => data.len(),
            Self::Dual { main, aux } => main.len() + aux.as_ref().map_or(0, Vec::len),
        }
    }
}

fn hardware_runtime_fallback_reason(
    codec_name: &str,
    payload_len: Option<usize>,
) -> Option<&'static str> {
    let is_hardware = matches!(codec_name, "VA-API H.264" | "Hardware H.264");
    if !is_hardware {
        return None;
    }

    match payload_len {
        None => Some("hardware encoder returned no frame"),
        Some(0) => Some("hardware encoder produced empty H.264 payload"),
        Some(_) => None,
    }
}

impl VideoEncoder {
    /// Encode a BGRA frame to H.264
    ///
    /// Returns the encoded frame data, or None if the encoder skipped the frame.
    fn encode_bgra(
        &mut self,
        bgra_data: &[u8],
        width: u32,
        height: u32,
        timestamp_ms: u64,
    ) -> Result<Option<EncodedVideoFrame>, crate::rdp::channels::graphics::egfx::EncoderError> {
        match self {
            VideoEncoder::Avc420(encoder) => encoder
                .encode_bgra(bgra_data, width, height, timestamp_ms)
                .map(|opt| opt.map(|frame| EncodedVideoFrame::Single(frame.data))),
            VideoEncoder::Avc444(encoder) => encoder
                .encode_bgra(bgra_data, width, height, timestamp_ms)
                .map(|opt| {
                    opt.map(|frame| EncodedVideoFrame::Dual {
                        main: frame.stream1_data,
                        aux: frame.stream2_data,
                    })
                }),
            #[cfg(feature = "vaapi")]
            VideoEncoder::Hardware(wrapper) => wrapper
                .0
                .encode_bgra(bgra_data, width, height, timestamp_ms)
                .map(|opt| opt.map(|frame| EncodedVideoFrame::Single(frame.data)))
                .map_err(|e| {
                    crate::rdp::channels::graphics::egfx::EncoderError::EncodeFailed(format!(
                        "Hardware encoder error: {e:?}"
                    ))
                }),
        }
    }

    /// Get codec name for logging
    fn codec_name(&self) -> &'static str {
        match self {
            VideoEncoder::Avc420(_) => "AVC420",
            VideoEncoder::Avc444(_) => "AVC444",
            #[cfg(feature = "vaapi")]
            VideoEncoder::Hardware(wrapper) => match wrapper.0.backend_name() {
                "vaapi" => "VA-API H.264",
                other => {
                    let _ = other;
                    "Hardware H.264"
                }
            },
        }
    }

    fn is_hardware(&self) -> bool {
        match self {
            VideoEncoder::Avc420(_) | VideoEncoder::Avc444(_) => false,
            #[cfg(feature = "vaapi")]
            VideoEncoder::Hardware(_) => true,
        }
    }

    /// Check if periodic IDR is due (non-consuming)
    fn is_periodic_idr_due(&self) -> bool {
        match self {
            VideoEncoder::Avc420(_) => false,
            VideoEncoder::Avc444(encoder) => encoder.is_periodic_idr_due(),
            #[cfg(feature = "vaapi")]
            VideoEncoder::Hardware(_) => false,
        }
    }
}

/// Frame rate regulator using token bucket algorithm
///
/// Ensures smooth video delivery by limiting frame rate to target FPS.
/// Uses token bucket to allow brief bursts while maintaining average rate.
struct FrameRateRegulator {
    /// Target frames per second
    target_fps: u32,
    /// Last frame send time
    last_frame_time: Instant,
    /// Token budget for burst handling (allows brief spikes)
    token_budget: f32,
    /// Maximum tokens that can accumulate
    max_tokens: f32,
}

impl FrameRateRegulator {
    fn new(target_fps: u32) -> Self {
        Self {
            target_fps,
            last_frame_time: Instant::now(),
            token_budget: 1.0,
            max_tokens: 2.0, // Allow 2-frame burst
        }
    }

    /// Check if a frame should be sent based on rate limiting
    /// Returns true if frame should be sent, false if it should be dropped
    fn should_send_frame(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame_time);

        // CRITICAL: Update last_frame_time on EVERY call, not just when sending
        // Otherwise dropped frames cause time to accumulate and earn too many tokens
        self.last_frame_time = now;

        // Add tokens based on elapsed time
        let tokens_earned = elapsed.as_secs_f32() * self.target_fps as f32;
        self.token_budget = (self.token_budget + tokens_earned).min(self.max_tokens);

        // Check if we have budget to send this frame
        if self.token_budget >= 1.0 {
            self.token_budget -= 1.0;
            true
        } else {
            // Drop frame - too fast
            false
        }
    }
}

/// RDP Display Handler
///
/// Provides the display size and update stream to IronRDP server.
/// Manages the video pipeline from PipeWire capture to RDP transmission.
///
/// # EGFX Support
///
/// When EGFX/H.264 is negotiated, frames are encoded with OpenH264 and sent
/// through the EGFX channel for better quality and compression. Falls back
/// to RemoteFX when H.264 is not available.
pub struct DisplayChannelHandler {
    /// Current desktop size
    size: Arc<RwLock<DesktopSize>>,

    /// PipeWire thread manager
    pipewire_thread: Arc<Mutex<PipeWireThreadManager>>,

    /// Bitmap converter for RDP format conversion
    bitmap_converter: Arc<Mutex<BitmapConverter>>,

    /// Display update sender (for creating update streams to IronRDP)
    /// Arc-wrapped so the pipeline task and IronRDP's clone share the same sender.
    /// On reconnection, updates() swaps this to a new channel — both sides must
    /// see the swap, or the pipeline sends to a dead channel.
    update_sender: Arc<tokio::sync::Mutex<mpsc::Sender<DisplayUpdate>>>,

    /// Display update receiver (wrapped for cloning)
    update_receiver: Arc<Mutex<Option<mpsc::Receiver<DisplayUpdate>>>>,

    /// Graphics queue sender (for priority multiplexing)
    graphics_tx: Option<mpsc::Sender<GraphicsFrame>>,

    /// Monitor configuration from streams
    stream_info: Vec<StreamInfo>,

    /// Explicit managed compositor socket used for output resizing.
    compositor_socket: Option<std::path::PathBuf>,

    // === EGFX/H.264 Support ===
    /// Shared GFX server handle for EGFX frame sending
    /// Populated by GfxFactory after channel attachment
    gfx_server_handle: Arc<RwLock<Option<GfxServerHandle>>>,

    /// Handler state for checking EGFX readiness
    gfx_handler_state: Arc<RwLock<Option<HandlerState>>>,

    /// Server event sender for routing EGFX messages
    /// Set after server is built (via set_server_event_sender)
    server_event_tx: Arc<RwLock<Option<mpsc::UnboundedSender<ServerEvent>>>>,

    /// Server configuration (for feature flags and settings)
    config: Arc<crate::config::Config>,

    /// Service registry for compositor-aware feature decisions
    service_registry: Arc<RuntimeCapabilities>,

    /// EGFX initialization flag - set to true when a new client needs EGFX setup
    ///
    /// This flag is checked by the pipeline to determine if EGFX surface setup
    /// (ResetGraphics, CreateSurface, MapSurfaceToOutput) needs to be performed.
    /// It's reset to `true` when a client reconnects so the new client gets
    /// proper EGFX initialization.
    egfx_needs_init: Arc<std::sync::atomic::AtomicBool>,

    /// Input handler reference for reconnection notification
    /// When client reconnects, we notify input handler to reset internal state
    input_handler: Arc<RwLock<Option<InputChannelHandler>>>,

    /// Clipboard manager reference for disconnect cleanup
    /// When client disconnects (detected via reconnection), clear clipboard state
    clipboard_manager: Arc<
        RwLock<
            Option<Arc<tokio::sync::Mutex<crate::rdp::channels::clipboard::ClipboardOrchestrator>>>,
        >,
    >,

    /// Latest client resize request. New requests replace the slot so resize
    /// bursts cannot leave the pipeline applying a stale intermediate size.
    pending_resize: Arc<std::sync::Mutex<Option<ResizeRequest>>>,

    /// Whether a client is actively connected and consuming frames.
    /// Set true on new connection (in `updates()`), false on disconnect.
    /// The pipeline loop checks this to avoid encoding/sending frames to nobody.
    client_active: Arc<std::sync::atomic::AtomicBool>,
    /// Health reporter for forwarding PipeWire stream state to health monitor
    health_reporter: Arc<RwLock<Option<crate::rdp::session::supervision::SessionStatusReporter>>>,
}

impl DisplayChannelHandler {
    fn should_rotate_rdp_frame_180() -> bool {
        static ROTATE: AtomicBool = AtomicBool::new(false);
        static INIT: AtomicBool = AtomicBool::new(false);

        if !INIT.swap(true, Ordering::SeqCst) {
            let enabled = std::env::var("WRDP_ROTATE_180")
                .map(|value| {
                    let value = value.trim().to_ascii_lowercase();
                    matches!(value.as_str(), "1" | "true" | "yes" | "on")
                })
                .unwrap_or(false);

            ROTATE.store(enabled, Ordering::SeqCst);
            if enabled {
                warn!("WRDP_ROTATE_180 enabled: rotating outgoing RDP frames by 180 degrees");
            }
        }

        ROTATE.load(Ordering::Relaxed)
    }

    fn should_flip_rdp_frame_vertical() -> bool {
        static FLIP: AtomicBool = AtomicBool::new(false);
        static INIT: AtomicBool = AtomicBool::new(false);

        if !INIT.swap(true, Ordering::SeqCst) {
            let enabled = std::env::var("WRDP_FLIP_VERTICAL")
                .map(|value| {
                    let value = value.trim().to_ascii_lowercase();
                    matches!(value.as_str(), "1" | "true" | "yes" | "on")
                })
                .unwrap_or(false);

            FLIP.store(enabled, Ordering::SeqCst);
            if enabled {
                warn!("WRDP_FLIP_VERTICAL enabled: vertically flipping outgoing RDP frames");
            }
        }

        FLIP.load(Ordering::Relaxed)
    }

    fn copy_frame_with_transform(
        frame: &VideoFrame,
        transform_name: &str,
        mut src_xy: impl FnMut(usize, usize, usize, usize) -> (usize, usize),
    ) -> VideoFrame {
        let bytes_per_pixel = frame.format.bytes_per_pixel() as usize;
        let width = frame.width as usize;
        let height = frame.height as usize;
        let src_stride = frame.stride as usize;
        let dst_stride = width.saturating_mul(bytes_per_pixel);

        if width == 0 || height == 0 || bytes_per_pixel == 0 {
            return frame.clone();
        }

        let required_src_len = src_stride.saturating_mul(height.saturating_sub(1)) + dst_stride;
        if frame.data.len() < required_src_len {
            warn!(
                "Skipping {} frame transform: frame buffer too small (len={}, required={})",
                transform_name,
                frame.data.len(),
                required_src_len
            );
            return frame.clone();
        }

        let mut transformed = vec![0u8; dst_stride * height];
        for y in 0..height {
            for x in 0..width {
                let (src_x, src_y) = src_xy(x, y, width, height);
                let src_offset = src_y * src_stride + src_x * bytes_per_pixel;
                let dst_offset = y * dst_stride + x * bytes_per_pixel;
                transformed[dst_offset..dst_offset + bytes_per_pixel]
                    .copy_from_slice(&frame.data[src_offset..src_offset + bytes_per_pixel]);
            }
        }

        let mut out = frame.clone();
        out.stride = dst_stride as u32;
        out.data = Arc::new(transformed);
        out.damage_regions.clear();
        out
    }

    fn flip_frame_vertical(frame: &VideoFrame) -> VideoFrame {
        Self::copy_frame_with_transform(frame, "vertical flip", |x, y, _width, height| {
            (x, height - 1 - y)
        })
    }

    fn rotate_frame_180(frame: &VideoFrame) -> VideoFrame {
        Self::copy_frame_with_transform(frame, "180-degree rotation", |x, y, width, height| {
            (width - 1 - x, height - 1 - y)
        })
    }

    fn pad_frame_to_aligned(
        data: &[u8],
        width: u32,
        height: u32,
        aligned_width: u32,
        aligned_height: u32,
    ) -> Vec<u8> {
        let bytes_per_pixel = 4;
        let src_stride = width * bytes_per_pixel;
        let dst_stride = aligned_width * bytes_per_pixel;
        let mut padded = vec![0u8; (aligned_width * aligned_height * bytes_per_pixel) as usize];

        for y in 0..height {
            let src_offset = (y * src_stride) as usize;
            let dst_offset = (y * dst_stride) as usize;
            padded[dst_offset..dst_offset + src_stride as usize]
                .copy_from_slice(&data[src_offset..src_offset + src_stride as usize]);

            if aligned_width > width {
                let last_pixel_src = src_offset + (src_stride - bytes_per_pixel) as usize;
                for x in width..aligned_width {
                    let dst_offset = (y * dst_stride + x * bytes_per_pixel) as usize;
                    padded[dst_offset..dst_offset + bytes_per_pixel as usize].copy_from_slice(
                        &data[last_pixel_src..last_pixel_src + bytes_per_pixel as usize],
                    );
                }
            }
        }

        if aligned_height > height {
            let last_row_offset = ((height - 1) * dst_stride) as usize;
            let last_row = padded[last_row_offset..last_row_offset + dst_stride as usize].to_vec();
            for y in height..aligned_height {
                let dst_offset = (y * dst_stride) as usize;
                padded[dst_offset..dst_offset + dst_stride as usize].copy_from_slice(&last_row);
            }
        }

        padded
    }

    fn allowed_resize(&self, raw_width: u32, raw_height: u32) -> Option<(u16, u16)> {
        use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;

        if !self.config.display.allow_resize {
            debug!("Dynamic resize disabled in config");
            return None;
        }

        let (width, height) = MonitorLayoutEntry::adjust_display_size(raw_width, raw_height);
        let requested_area = u64::from(width) * u64::from(height);
        const MAX_AREA: u64 = 3840 * 2400;
        if requested_area > MAX_AREA {
            warn!(
                "Requested area {width}x{height} = {requested_area} exceeds max {MAX_AREA} pixels"
            );
            return None;
        }

        let width = width as u16;
        let height = height as u16;
        if !self.config.display.allowed_resolutions.is_empty() {
            let target = format!("{width}x{height}");
            if !self.config.display.allowed_resolutions.contains(&target) {
                debug!("Resolution {target} not in allowed list");
                return None;
            }
        }

        Some((width, height))
    }

    async fn resize_managed_compositor(&self, width: u16, height: u16) -> Result<()> {
        let output =
            std::env::var("WRDP_HEADLESS_OUTPUT").unwrap_or_else(|_| "HEADLESS-1".to_string());
        let mode = format!("{width}x{height}");
        let mut command = tokio::process::Command::new("wlr-randr");
        command.args(["--output", output.as_str(), "--custom-mode", mode.as_str()]);

        if let Some(socket) = &self.compositor_socket {
            let runtime_dir = socket
                .parent()
                .context("managed compositor socket has no runtime directory")?;
            let display = socket
                .file_name()
                .context("managed compositor socket has no display name")?;
            command
                .env("XDG_RUNTIME_DIR", runtime_dir)
                .env("WAYLAND_DISPLAY", display);
        } else {
            command.env(
                "WAYLAND_DISPLAY",
                std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_string()),
            );
        }

        let status = command.status().await?;

        if !status.success() {
            anyhow::bail!("wlr-randr exited with status {status}");
        }
        Ok(())
    }

    fn crop_frame_to_size(frame: &VideoFrame, width: u16, height: u16) -> VideoFrame {
        let target_width = u32::from(width).min(frame.width);
        let target_height = u32::from(height).min(frame.height);

        let bytes_per_pixel = frame.format.bytes_per_pixel() as u32;
        let row_bytes = target_width * bytes_per_pixel;

        // Encoders and damage detection consume tightly packed rows. Preserve
        // the original allocation only when it is already compact; otherwise
        // strip source-row padding even when no geometric crop is needed.
        if target_width == frame.width && target_height == frame.height && frame.stride == row_bytes
        {
            return frame.clone();
        }
        // Keep cropped video frames compact. EGFX AVC/Planar encoders validate
        // data_len == width * height * bytes_per_pixel; leaking stride padding
        // here causes Android-sized frames such as 1596×768 to become 1600×768
        // worth of bytes while still being advertised as 1596×768.
        let target_stride = row_bytes;
        let mut cropped = vec![0u8; (row_bytes * target_height) as usize];

        for y in 0..target_height {
            let src_offset = (y * frame.stride) as usize;
            let dst_offset = (y * row_bytes) as usize;
            cropped[dst_offset..dst_offset + row_bytes as usize]
                .copy_from_slice(&frame.data[src_offset..src_offset + row_bytes as usize]);
        }

        let mut out = frame.clone();
        out.width = target_width;
        out.height = target_height;
        out.stride = target_stride;
        out.data = Arc::new(cropped);
        out.damage_regions.clear();
        out
    }

    /// Create display handler with a direct frame channel (no PipeWire fd).
    ///
    /// Used by portal-generic where screencopy delivers frames via mpsc channel
    /// rather than through PipeWire's buffer sharing mechanism.
    #[expect(
        clippy::too_many_arguments,
        reason = "display handler needs pipeline components at construction"
    )]
    pub async fn new_direct(
        initial_width: u16,
        initial_height: u16,
        raw_rx: std::sync::mpsc::Receiver<crate::desktop::pipewire::frame::RawFrameData>,
        stream_info: Vec<StreamInfo>,
        compositor_socket: Option<std::path::PathBuf>,
        graphics_tx: Option<mpsc::Sender<GraphicsFrame>>,
        gfx_server_handle: Option<Arc<RwLock<Option<GfxServerHandle>>>>,
        gfx_handler_state: Option<Arc<RwLock<Option<HandlerState>>>>,
        config: Arc<crate::config::Config>,
        service_registry: Arc<RuntimeCapabilities>,
    ) -> Result<Self> {
        let size = Arc::new(RwLock::new(DesktopSize {
            width: initial_width,
            height: initial_height,
        }));

        let pipewire_thread = Arc::new(Mutex::new(PipeWireThreadManager::new_direct(
            raw_rx,
            initial_width as u32,
            initial_height as u32,
        )?));

        info!(
            "Display handler created (direct channel): {}x{}, {} streams",
            initial_width,
            initial_height,
            stream_info.len(),
        );

        let bitmap_converter = Arc::new(Mutex::new(BitmapConverter::new(
            initial_width,
            initial_height,
        )));

        let (update_sender, update_receiver) = mpsc::channel(64);
        let update_sender = Arc::new(tokio::sync::Mutex::new(update_sender));
        let update_receiver = Arc::new(Mutex::new(Some(update_receiver)));

        let gfx_server_handle = gfx_server_handle.unwrap_or_else(|| Arc::new(RwLock::new(None)));
        let gfx_handler_state = gfx_handler_state.unwrap_or_else(|| Arc::new(RwLock::new(None)));

        Ok(Self {
            size,
            pipewire_thread,
            bitmap_converter,
            update_sender,
            update_receiver,
            graphics_tx,
            stream_info,
            compositor_socket,
            gfx_server_handle,
            gfx_handler_state,
            server_event_tx: Arc::new(RwLock::new(None)),
            config,
            service_registry,
            egfx_needs_init: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            input_handler: Arc::new(RwLock::new(None)),
            clipboard_manager: Arc::new(RwLock::new(None)),
            pending_resize: Arc::new(std::sync::Mutex::new(None)),
            client_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            health_reporter: Arc::new(RwLock::new(None)),
        })
    }

    /// Set input handler reference for reconnection notifications
    ///
    /// Must be called after input handler is created to enable reconnection reset.
    pub async fn set_input_handler(
        &self,
        handler: Arc<crate::rdp::server::input_handler::InputChannelHandler>,
    ) {
        *self.input_handler.write().await = Some((*handler).clone());
        info!("Input handler reference set for reconnection notifications");
    }

    /// Wire the health reporter so PipeWire stream state events propagate
    /// to the session health monitor.
    pub async fn set_health_reporter(
        &self,
        reporter: crate::rdp::session::supervision::SessionStatusReporter,
    ) {
        *self.health_reporter.write().await = Some(reporter);
    }

    /// Set clipboard manager reference for disconnect cleanup
    ///
    /// When client disconnects (detected via reconnection), the display handler
    /// will clear clipboard state to prevent stale operations.
    pub async fn set_clipboard_manager(
        &self,
        manager: Arc<tokio::sync::Mutex<crate::rdp::channels::clipboard::ClipboardOrchestrator>>,
    ) {
        *self.clipboard_manager.write().await = Some(manager);
        info!("Clipboard manager reference set for disconnect cleanup");
    }

    /// Signal that the client has disconnected.
    ///
    /// The pipeline loop checks `client_active` and skips encoding/sending when
    /// no client is connected. PipeWire frames are still drained to keep the
    /// stream healthy, but no CPU is wasted on encoding or queue pressure.
    pub fn on_client_disconnect(&self) {
        self.client_active
            .store(false, std::sync::atomic::Ordering::SeqCst);
        info!("Client disconnect signaled to pipeline - frame processing paused");
    }

    /// Release references that keep connection-owned input/session resources alive.
    pub async fn release_connection_resources(&self) {
        self.on_client_disconnect();
        self.input_handler.write().await.take();
        self.clipboard_manager.write().await.take();
        self.shutdown_pipewire().await;
    }

    /// Whether a client is currently marked active by the display pipeline.
    ///
    /// mstsc can open extra short-lived probe/retry TCP connections while the
    /// authenticated session is active. Those failed probe connections must not
    /// clear the active session's pipeline state.
    pub fn is_client_active(&self) -> bool {
        self.client_active.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Set graphics queue sender for priority multiplexing
    ///
    /// When set, frames will be routed through the graphics queue instead of
    /// directly to IronRDP's DisplayUpdate channel.
    pub fn set_graphics_queue(&mut self, sender: mpsc::Sender<GraphicsFrame>) {
        info!("Graphics queue sender configured for priority multiplexing");
        self.graphics_tx = Some(sender);
    }

    /// Set the server event sender for EGFX message routing
    ///
    /// This must be called after the RDP server is built, passing a clone of
    /// `event_sender()` from the server. Required for EGFX frame sending.
    pub async fn set_server_event_sender(&self, sender: mpsc::UnboundedSender<ServerEvent>) {
        *self.server_event_tx.write().await = Some(sender);
        info!("Server event sender configured for EGFX routing");
    }

    /// Reset the display update channel for a new client connection
    ///
    /// Called when a client disconnects to allow the next client to claim
    /// display updates. Creates a fresh sender/receiver pair.
    pub async fn reset_update_channel(&mut self) {
        let (new_sender, new_receiver) = mpsc::channel(64);
        *self.update_sender.lock().await = new_sender;
        *self.update_receiver.lock().await = Some(new_receiver);
        debug!("Display update channel reset for new client");
    }

    /// Check if EGFX is ready for frame sending
    ///
    /// Returns true if:
    /// - GFX server handle is available
    /// - Handler state indicates readiness (capabilities negotiated)
    /// - Server event sender is configured
    pub async fn is_egfx_ready(&self) -> bool {
        if self.server_event_tx.read().await.is_none() {
            return false;
        }

        if self.gfx_server_handle.read().await.is_none() {
            return false;
        }

        if let Some(state) = self.gfx_handler_state.read().await.as_ref() {
            state.is_ready
        } else {
            false
        }
    }

    /// Return the explicit negotiated EGFX mode for the connected client.
    pub async fn negotiated_egfx_mode(&self) -> Option<NegotiatedEgfxMode> {
        self.gfx_handler_state
            .read()
            .await
            .as_ref()
            .and_then(|state| state.negotiated_mode)
    }

    /// Check if AVC/H.264 is available for the negotiated client mode.
    pub async fn is_avc_supported(&self) -> bool {
        self.negotiated_egfx_mode()
            .await
            .is_some_and(NegotiatedEgfxMode::uses_avc)
    }

    /// Get a descriptive reason for why EGFX is not ready
    ///
    /// Returns a human-readable string explaining the current wait state.
    /// Useful for debugging connection/negotiation issues.
    pub async fn egfx_wait_reason(&self) -> &'static str {
        if self.server_event_tx.read().await.is_none() {
            return "waiting for client connection";
        }

        if self.gfx_server_handle.read().await.is_none() {
            return "client connected, waiting for EGFX channel";
        }

        if let Some(state) = self.gfx_handler_state.read().await.as_ref() {
            if !state.is_ready {
                return "EGFX channel open, negotiating capabilities";
            }
            if let Some(mode) = state.negotiated_mode {
                return match mode {
                    NegotiatedEgfxMode::Avc420 | NegotiatedEgfxMode::Avc444 => {
                        "EGFX ready, AVC/H.264 negotiated"
                    }
                    NegotiatedEgfxMode::Bitmap => {
                        "EGFX unavailable by policy; using FastPath bitmap"
                    }
                };
            }
        } else {
            return "EGFX channel open, initializing handler state";
        }

        "ready" // Should not reach here if is_egfx_ready() is false
    }

    /// Update the desktop size
    ///
    /// Called when monitor configuration changes or client requests resize.
    pub async fn update_size(&self, width: u16, height: u16) {
        let mut size = self.size.write().await;
        size.width = width;
        size.height = height;
        debug!("Updated display size to {}x{}", width, height);

        let update = DisplayUpdate::Resize(DesktopSize { width, height });
        // Reconnection swaps this shared sender. Do not retain the mutex while
        // waiting for channel capacity or the swap can be blocked indefinitely.
        let sender = self.update_sender.lock().await.clone();
        if let Err(e) = sender.send(update).await {
            warn!("Failed to send resize update: {}", e);
        }
    }

    /// Get a shared reference to the update sender for graphics drain task
    ///
    /// This is used by the Phase 1 multiplexer to get access to the IronRDP update channel.
    /// Returns an Arc so the drain task and the handler share the same sender — when the
    /// channel is recreated on reconnection, both sides see the new sender.
    pub fn get_update_sender(&self) -> Arc<tokio::sync::Mutex<mpsc::Sender<DisplayUpdate>>> {
        Arc::clone(&self.update_sender)
    }

    /// Get shared EGFX capability state for Android-only client quirk gating.
    pub fn get_gfx_handler_state(&self) -> Arc<RwLock<Option<HandlerState>>> {
        Arc::clone(&self.gfx_handler_state)
    }

    /// Shutdown PipeWire thread explicitly
    ///
    /// Must be called during server shutdown to ensure PipeWire thread exits.
    /// The PipeWireThreadManager lives in Arc<Mutex<>> which may have multiple
    /// references (e.g., from spawned pipeline task), so Drop may not trigger
    /// until after runtime shutdown.
    ///
    /// Calling this method sends shutdown signals directly to the PipeWire thread,
    /// ensuring immediate cleanup regardless of reference count.
    pub async fn shutdown_pipewire(&self) {
        info!("Shutting down PipeWire thread...");
        let mut thread_mgr = self.pipewire_thread.lock().await;
        if let Err(e) = thread_mgr.shutdown() {
            warn!("PipeWire shutdown error: {}", e);
        } else {
            info!("✅ PipeWire thread shut down successfully");
        }
    }

    /// Start the video pipeline
    ///
    /// This spawns a background task that continuously captures frames from PipeWire,
    /// processes them, and sends them via either EGFX (H.264) or RemoteFX path.
    ///
    /// # Path Selection
    ///
    /// - **EGFX/H.264**: When client negotiates AVC420 support, frames are encoded
    ///   with OpenH264 and sent through the EGFX channel for better quality.
    /// - **RemoteFX**: Fallback path when H.264 is not available, converts to
    ///   bitmap and sends through standard display update channel.
    #[expect(clippy::expect_used, reason = "mutex poisoning is unrecoverable")]
    pub fn start_pipeline(self: Arc<Self>) {
        let handler = Arc::clone(&self);

        tokio::spawn(async move {
            info!("🎬 Starting display update pipeline task");

            // === ADAPTIVE FPS CONTROLLER (Premium Feature) ===
            // Dynamically adjusts frame rate based on screen activity:
            // - Static screen: 5 FPS (saves CPU/bandwidth)
            // - Low activity (typing): 15 FPS
            // - Medium activity (scrolling): 20 FPS
            // - High activity (video): 30 FPS
            //
            // SERVICE-AWARE: Only enable when damage tracking service is available
            // (without it, adaptive FPS has no activity detection signal)
            let service_supports_adaptive_fps = self.service_registry.should_enable_adaptive_fps();
            let adaptive_fps_enabled =
                self.config.performance.adaptive_fps.enabled && service_supports_adaptive_fps;
            if self.config.performance.adaptive_fps.enabled && !service_supports_adaptive_fps {
                info!("⚠️ Adaptive FPS disabled: damage tracking service unavailable");
            }
            let adaptive_fps_config =
                crate::rdp::channels::graphics::performance::AdaptiveFpsConfig {
                    enabled: adaptive_fps_enabled,
                    min_fps: self.config.performance.adaptive_fps.min_fps,
                    max_fps: self.config.performance.adaptive_fps.max_fps,
                    high_activity_threshold: self
                        .config
                        .performance
                        .adaptive_fps
                        .high_activity_threshold,
                    medium_activity_threshold: self
                        .config
                        .performance
                        .adaptive_fps
                        .medium_activity_threshold,
                    low_activity_threshold: self
                        .config
                        .performance
                        .adaptive_fps
                        .low_activity_threshold,
                    ..Default::default()
                };
            let mut adaptive_fps = AdaptiveFpsController::new(adaptive_fps_config);

            // === LATENCY GOVERNOR (Premium Feature) ===
            // Controls encoding latency vs quality trade-off:
            // - Interactive (<50ms): Gaming, CAD - encode immediately
            // - Balanced (<100ms): General desktop - smart batching
            // - Quality (<300ms): Photo/video editing - accumulate for quality
            //
            // SERVICE-AWARE: ExplicitSync service affects frame pacing accuracy
            let explicit_sync = self.service_registry.explicit_sync;
            let latency_mode = match self.config.performance.latency.mode.as_str() {
                "interactive" => LatencyMode::Interactive,
                "quality" => LatencyMode::Quality,
                _ => LatencyMode::Balanced,
            };
            let mut latency_governor = LatencyGovernor::new(latency_mode);

            // Log service-aware performance feature status
            let damage_hints = self.service_registry.damage_hints;
            let dmabuf = self.service_registry.dmabuf;
            info!(
                "🎛️ Performance features: adaptive_fps={}, latency_mode={:?}",
                adaptive_fps_enabled, latency_mode
            );
            info!(
                "   Services: damage_tracking={}, explicit_sync={}, dmabuf={}",
                damage_hints, explicit_sync, dmabuf
            );

            // Frame regulator fallback (used when adaptive FPS is disabled)
            // Uses configured max_fps (default: 30, can be 60 for high-performance mode)
            let legacy_fps = self.config.performance.adaptive_fps.max_fps;
            let mut frame_regulator = FrameRateRegulator::new(legacy_fps);
            let mut frames_sent = 0u64;
            let mut frames_dropped = 0u64;
            let mut egfx_frames_sent = 0u64;

            let mut loop_iterations = 0u64;

            // EGFX/H.264 encoder - created lazily when EGFX becomes ready
            // Supports both AVC420 (4:2:0) and AVC444 (4:4:4) based on client negotiation
            // NOTE: These are reset when egfx_needs_init transitions from true to false
            let mut video_encoder: Option<VideoEncoder> = None;
            let mut egfx_sender: Option<EgfxChannelSender> = None;
            let mut h264_encoder_config: Option<EncoderConfig> = None;
            #[cfg(feature = "vaapi")]
            let mut hardware_encoding_runtime_disabled = false;
            // IronRDP Planar encoder for EGFX Planar path (used when AVC is disabled and RFX unsupported)
            // AVC444 vs AVC420 determined by VideoEncoder enum variant match, not a flag

            // Force first frame after initialization - bypasses damage detection
            // Without this, reconnecting clients see black screen until mouse moves
            // because damage detection reports 0% change on first frame (no previous data)
            let mut force_first_frame = false;

            // Last-frame cache: holds the most recent PipeWire frame for replay on
            // EGFX initialization. Portal ScreenCast is damage-driven — PipeWire only
            // delivers frames when screen content changes. On a static desktop, the
            // initial burst of frames arrives before any RDP client connects (drained
            // at the client_active gate). By the time EGFX negotiation completes, there
            // are no new frames to encode and the client sees nothing.
            //
            // This cache ensures every client gets at least one H.264 frame (the current
            // desktop state) immediately after EGFX becomes ready, regardless of whether
            // PipeWire has pending frames.
            //
            // Cost: one Arc<Vec<u8>> reference (~8MB at 1080p BGRA). VideoFrame.data is
            // Arc-wrapped, so clone is a refcount bump — no pixel data is copied.
            //
            // Future backend-specific refresh hooks can provide fresher capture
            // snapshots than this cache. Until then, this cached frame remains
            // the universal fallback when no fresh frame arrives during connect.
            let mut cached_frame: Option<crate::pipewire::VideoFrame> = None;

            // === DAMAGE DETECTION (Config-controlled) ===
            // Detects changed screen regions to skip unchanged frames (90%+ bandwidth reduction for static content)
            // All parameters now configurable via wrdp.ini [damage_tracking] section
            // See DamageTrackingConfig documentation for sensitivity tuning guidance
            let damage_config = DamageConfig {
                tile_size: self.config.damage_tracking.tile_size,
                diff_threshold: self.config.damage_tracking.diff_threshold,
                pixel_threshold: self.config.damage_tracking.pixel_threshold,
                merge_distance: self.config.damage_tracking.merge_distance,
                min_region_area: self.config.damage_tracking.min_region_area,
            };

            let mut damage_detector_opt = if self.config.damage_tracking.enabled {
                debug!(
                    "Damage tracking ENABLED: tile_size={}, threshold={:.2}, pixel_threshold={}, merge_distance={}, min_region_area={}",
                    damage_config.tile_size,
                    damage_config.diff_threshold,
                    damage_config.pixel_threshold,
                    damage_config.merge_distance,
                    damage_config.min_region_area
                );
                Some(DamageDetector::new(damage_config))
            } else {
                debug!("🎯 Damage tracking DISABLED via config");
                None
            };

            let mut frames_skipped_damage = 0u64; // Frames skipped due to no damage

            // === FRAME STALL DETECTION ===
            // Track when we last received a frame from PipeWire. If the stream
            // is active but no frames arrive for 3+ seconds, report degradation
            // to the health monitor. Recovery is reported when frames resume.
            let mut last_frame_time = std::time::Instant::now();
            let mut video_stall_reported = false;
            let stall_threshold = std::time::Duration::from_secs(3);

            // Zero-frame detection: if we never receive ANY frame within 10 seconds
            // of session start, something is fundamentally wrong (e.g., ext-capture
            // handshake completed but compositor never delivers frames).
            let mut session_start = std::time::Instant::now();
            let mut first_frame_received = false;
            let mut zero_frame_reported = false;

            // EGFX readiness timeout: if EGFX hasn't become ready within 5 seconds
            // of the first PipeWire frame, assume the client doesn't support DVC or
            // EGFX negotiation failed. Bypass the EGFX gate and deliver frames via
            // FastPath bitmap only. Without this, clients without DVC get zero frames.
            let egfx_timeout = std::time::Duration::from_secs(5);
            let mut egfx_gate_bypassed = false;
            let mut was_client_active = false;
            // Direct-channel capture (portal-generic) learns the authoritative
            // source dimensions from frames, not PipeWire stream metadata. Track
            // the live frame size so desktop size, EGFX surfaces, and input
            // coordinate normalization stay aligned.
            let mut last_direct_geometry: Option<(u32, u32)> = None;
            let zero_frame_threshold = std::time::Duration::from_secs(10);

            // === PTS INTERVAL TRACKING ===
            // Track PipeWire presentation timestamps to measure actual frame
            // delivery cadence. Reported in the heartbeat log.
            let mut last_pts_nsec: u64 = 0;
            let mut pts_interval_sum_ms: f64 = 0.0;
            let mut pts_interval_count: u64 = 0;
            let mut pts_interval_min_ms: f64 = f64::MAX;
            let mut pts_interval_max_ms: f64 = 0.0;

            loop {
                loop_iterations += 1;
                if loop_iterations.is_multiple_of(1000) {
                    if pts_interval_count > 0 {
                        let avg_ms = pts_interval_sum_ms / pts_interval_count as f64;
                        debug!(
                            "Display pipeline heartbeat: {} iterations, sent {} (egfx: {}), dropped {}, skipped_damage {}, pts_interval {:.1}/{:.1}/{:.1}ms (min/avg/max, n={})",
                            loop_iterations,
                            frames_sent,
                            egfx_frames_sent,
                            frames_dropped,
                            frames_skipped_damage,
                            pts_interval_min_ms,
                            avg_ms,
                            pts_interval_max_ms,
                            pts_interval_count,
                        );
                        // Reset for next window
                        pts_interval_sum_ms = 0.0;
                        pts_interval_count = 0;
                        pts_interval_min_ms = f64::MAX;
                        pts_interval_max_ms = 0.0;
                    } else {
                        debug!(
                            "Display pipeline heartbeat: {} iterations, sent {} (egfx: {}), dropped {}, skipped_damage {}",
                            loop_iterations,
                            frames_sent,
                            egfx_frames_sent,
                            frames_dropped,
                            frames_skipped_damage
                        );
                    }
                }

                // === CLIENT-INITIATED RESIZE ===
                // Atomically take the latest request. Bursty edge dragging
                // replaces this slot and therefore converges on the final size.
                let latest_resize = handler
                    .pending_resize
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.take());
                if let Some(req) = latest_resize {
                    info!(
                        "Direct-channel resize requested: {}x{}; updating compositor, RDP desktop, and EGFX surfaces",
                        req.width, req.height
                    );

                    if let Err(e) = handler
                        .resize_managed_compositor(req.width, req.height)
                        .await
                    {
                        warn!(
                            "Failed to resize managed compositor output; using frame crop fallback: {e}"
                        );
                    }
                    handler.update_size(req.width, req.height).await;
                    {
                        let mut converter = handler.bitmap_converter.lock().await;
                        *converter = BitmapConverter::new(req.width, req.height);
                    }
                    if let Some(ref mut detector) = damage_detector_opt {
                        detector.invalidate();
                    }

                    // Force the next frame through a fresh EGFX ResetGraphics /
                    // CreateSurface sequence so the server follows client-side
                    // window resizes instead of merely recording them.
                    video_encoder = None;
                    egfx_sender = None;
                    force_first_frame = true;
                    handler
                        .egfx_needs_init
                        .store(true, std::sync::atomic::Ordering::SeqCst);

                    let (stream_w, stream_h) = last_direct_geometry
                        .unwrap_or((u32::from(req.width), u32::from(req.height)));
                    let input_handler = handler.input_handler.read().await.clone();
                    if let Some(input_handler) = input_handler
                        && let Err(e) = input_handler
                            .update_primary_stream_mapping(
                                u32::from(req.width),
                                u32::from(req.height),
                                stream_w,
                                stream_h,
                            )
                            .await
                    {
                        warn!("Failed to update direct input mapping after resize: {e}");
                    }
                }

                let frame = {
                    let thread_mgr = handler.pipewire_thread.lock().await;

                    // Forward PipeWire stream state changes to health monitor
                    if let Some(ref reporter) = *handler.health_reporter.read().await {
                        for event in thread_mgr.drain_state_events() {
                            let health_state = match event.state {
                                crate::desktop::pipewire::PwStreamState::Streaming => {
                                    crate::rdp::session::supervision::GraphicsStreamState::Streaming
                                }
                                crate::desktop::pipewire::PwStreamState::Paused => {
                                    crate::rdp::session::supervision::GraphicsStreamState::Paused
                                }
                                crate::desktop::pipewire::PwStreamState::Error(ref msg) => {
                                    warn!("PipeWire stream error: {}", msg);
                                    crate::rdp::session::supervision::GraphicsStreamState::Error
                                }
                                crate::desktop::pipewire::PwStreamState::Unconnected => {
                                    warn!(
                                        "PipeWire stream disconnected - screen capture unavailable"
                                    );
                                    if std::env::var("WAYLAND_DISPLAY").is_err() {
                                        warn!(
                                            "WAYLAND_DISPLAY is not set - this is likely the cause"
                                        );
                                    }
                                    continue;
                                }
                                // Connecting is transient -- not a health event
                                crate::desktop::pipewire::PwStreamState::Connecting => continue,
                            };
                            reporter.report(crate::rdp::session::supervision::SessionStatusEvent::GraphicsStreamStateChanged {
                                state: health_state,
                            });
                        }
                    }

                    let mut latest = thread_mgr.try_recv_frame();
                    if latest.is_some() {
                        let mut drained = 0u64;
                        while let Some(next) = thread_mgr.try_recv_frame() {
                            latest = Some(next);
                            drained = drained.saturating_add(1);
                        }
                        if drained > 0 && (drained >= 30 || loop_iterations.is_multiple_of(300)) {
                            debug!("portal-generic: drained {drained} stale queued direct frames");
                        }
                    }
                    latest
                };

                let frame = match frame {
                    Some(f) => {
                        let frame_geometry = (f.width.max(1), f.height.max(1));
                        if last_direct_geometry != Some(frame_geometry) {
                            let frame_w = frame_geometry.0.min(u32::from(u16::MAX)) as u16;
                            let frame_h = frame_geometry.1.min(u32::from(u16::MAX)) as u16;
                            let current_size = *handler.size.read().await;
                            let target_w = current_size.width.min(frame_w).max(1);
                            let target_h = current_size.height.min(frame_h).max(1);

                            info!(
                                "portal-generic: direct frame geometry {}x{} -> desktop {}x{} / input stream {}x{}",
                                f.width, f.height, target_w, target_h, frame_w, frame_h
                            );

                            {
                                let mut converter = handler.bitmap_converter.lock().await;
                                *converter = BitmapConverter::new(target_w, target_h);
                            }
                            if let Some(ref mut detector) = damage_detector_opt {
                                detector.invalidate();
                            }

                            // Force EGFX ResetGraphics/CreateSurface on the new size.
                            video_encoder = None;
                            egfx_sender = None;
                            handler
                                .egfx_needs_init
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                            handler.update_size(target_w, target_h).await;

                            let input_handler = handler.input_handler.read().await.clone();
                            if let Some(input_handler) = input_handler
                                && let Err(e) = input_handler
                                    .update_primary_stream_mapping(
                                        u32::from(target_w),
                                        u32::from(target_h),
                                        frame_geometry.0,
                                        frame_geometry.1,
                                    )
                                    .await
                            {
                                warn!("Failed to update direct input mapping: {e}");
                            }
                            last_direct_geometry = Some(frame_geometry);
                        }

                        // Always cache the latest frame for replay on EGFX init.
                        // Clone is cheap: VideoFrame.data is Arc<Vec<u8>>.
                        cached_frame = Some(f.clone());
                        last_frame_time = std::time::Instant::now();

                        // Track PTS intervals for heartbeat diagnostics
                        if f.pts > 0 && last_pts_nsec > 0 && f.pts > last_pts_nsec {
                            let interval_ms = (f.pts - last_pts_nsec) as f64 / 1_000_000.0;
                            pts_interval_sum_ms += interval_ms;
                            pts_interval_count += 1;
                            if interval_ms < pts_interval_min_ms {
                                pts_interval_min_ms = interval_ms;
                            }
                            if interval_ms > pts_interval_max_ms {
                                pts_interval_max_ms = interval_ms;
                            }
                        }
                        if f.pts > 0 {
                            last_pts_nsec = f.pts;
                        }

                        // Mark that we've received at least one frame
                        first_frame_received = true;

                        // Report recovery if we previously flagged a stall
                        if video_stall_reported {
                            video_stall_reported = false;
                            if let Some(ref reporter) = *handler.health_reporter.read().await {
                                reporter.report_aspect_health(
                                    crate::rdp::session::supervision::SessionAspectId::Graphics,
                                    crate::rdp::session::supervision::AspectHealth::Healthy,
                                );
                            }
                        }

                        // Drain PipeWire frames even when no client is connected,
                        // but skip all encoding and sending to avoid wasted work
                        let client_now_active = handler
                            .client_active
                            .load(std::sync::atomic::Ordering::Relaxed);
                        if !client_now_active {
                            was_client_active = false;
                            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                            continue;
                        }

                        // Reset per-connection state on reconnection.
                        // The EGFX gate timeout must count from connection start,
                        // not server start — otherwise after the first 5s of uptime,
                        // every subsequent client bypasses the gate immediately and
                        // gets FastPath bitmaps instead of EGFX.
                        if !was_client_active {
                            was_client_active = true;
                            session_start = std::time::Instant::now();
                            egfx_gate_bypassed = false;
                            first_frame_received = false;
                            zero_frame_reported = false;
                            frames_sent = 0;
                            frames_dropped = 0;
                            egfx_frames_sent = 0;
                            video_encoder = None;
                            egfx_sender = None;
                            // Clear cached frame from the earlier session. The cached frame
                            // was captured for a different client (possibly different
                            // codec/size). Replaying it into the new EGFX surface
                            // before proper init causes garbled display on cross-client
                            // reconnection (e.g. Windows→Android).
                            cached_frame = None;
                            handler
                                .egfx_needs_init
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                            // Reset only per-pipeline EGFX objects here. The shared
                            // handler state/server handle are connection-owned by
                            // EgfxChannelFactory::build_server_with_handle(); clearing them
                            // here races with fast EGFX capability negotiation (Android
                            // AVC-disabled/Planar) and prevents Planar init.
                            info!("Pipeline state reset for new client connection (cache cleared)");
                        }
                        debug!("Received frame from PipeWire");
                        f
                    }
                    None => {
                        // Stall detection: if we previously received frames (cached_frame
                        // exists) and haven't gotten one for 3+ seconds, the stream may be
                        // stuck. Static desktops normally produce no frames (damage-driven),
                        // so we only flag this after we've seen at least one frame.
                        if cached_frame.is_some() && !video_stall_reported {
                            let elapsed = last_frame_time.elapsed();
                            if elapsed > stall_threshold {
                                video_stall_reported = true;
                                if let Some(ref reporter) = *handler.health_reporter.read().await {
                                    reporter.report_aspect_health(
                                        crate::rdp::session::supervision::SessionAspectId::Graphics,
                                        crate::rdp::session::supervision::AspectHealth::Degraded(
                                            format!("no frames for {}ms", elapsed.as_millis()),
                                        ),
                                    );
                                }
                            }
                        }

                        // Zero-frame detection: if no frame has EVER arrived since session
                        // start, the capture protocol may be non-functional (e.g., ext-capture
                        // on a compositor with incomplete implementation).
                        if !first_frame_received && !zero_frame_reported {
                            let since_start = session_start.elapsed();
                            if since_start > zero_frame_threshold {
                                zero_frame_reported = true;
                                tracing::warn!(
                                    elapsed_ms = since_start.as_millis() as u64,
                                    "No video frames received since session start"
                                );
                                if let Some(ref reporter) = *handler.health_reporter.read().await {
                                    reporter.report_aspect_health(
                                        crate::rdp::session::supervision::SessionAspectId::Graphics,
                                        crate::rdp::session::supervision::AspectHealth::Failed(
                                            format!(
                                                "capture never delivered frames ({}ms)",
                                                since_start.as_millis()
                                            ),
                                        ),
                                    );
                                }
                            }
                        }

                        // No fresh frame from PipeWire. Check if we should replay
                        // the cached frame for EGFX initialization.
                        //
                        // Portal ScreenCast is damage-driven: on a static desktop,
                        // try_recv_frame() returns None indefinitely. Without this
                        // replay, EGFX-ready clients never receive their first H.264
                        // frame and show a black screen until something moves.
                        let client_waiting = handler
                            .client_active
                            .load(std::sync::atomic::Ordering::Relaxed);

                        // Also reset per-connection state from the None arm,
                        // in case PipeWire hasn't delivered a frame yet
                        if client_waiting && !was_client_active {
                            was_client_active = true;
                            session_start = std::time::Instant::now();
                            egfx_gate_bypassed = false;
                            first_frame_received = false;
                            zero_frame_reported = false;
                            frames_sent = 0;
                            frames_dropped = 0;
                            egfx_frames_sent = 0;
                            video_encoder = None;
                            egfx_sender = None;
                            // Clear cached frame from the earlier session (same reason
                            // as the Some-arm: avoid replaying stale frames across
                            // different clients/codecs).
                            cached_frame = None;
                            handler
                                .egfx_needs_init
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                            // Reset only per-pipeline EGFX objects here. The shared
                            // handler state/server handle are connection-owned by
                            // EgfxChannelFactory::build_server_with_handle(); clearing them
                            // here races with fast EGFX capability negotiation (Android
                            // AVC-disabled/Planar) and prevents Planar init.
                            info!(
                                "Pipeline state reset for new client connection (no-frame path, cache cleared)"
                            );
                        }

                        let needs_init = handler
                            .egfx_needs_init
                            .load(std::sync::atomic::Ordering::Relaxed);

                        let should_replay_for_egfx =
                            client_waiting && needs_init && handler.is_egfx_ready().await;

                        if should_replay_for_egfx {
                            if let Some(ref cached) = cached_frame {
                                info!(
                                    "📦 Replaying cached frame for EGFX init ({}x{}, frame {})",
                                    cached.width, cached.height, cached.frame_id
                                );
                                cached.clone()
                            } else {
                                // No cached frame yet (server just started, PipeWire
                                // hasn't delivered any frames). Wait for first frame.
                                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                                continue;
                            }
                        } else {
                            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                            continue;
                        }
                    }
                };

                // Use the client-negotiated desktop size for display encoding.
                // PipeWire may keep producing the compositor/source size after a
                // mobile client rotates or requests a smaller portrait/landscape
                // desktop. If we encode/send the raw source dimensions while EGFX
                // ResetGraphics/CreateSurface use the client size, Android RD
                // Client renders corrupted pixels or a blank surface. Crop to the
                // negotiated size before damage detection, encoder setup, surface
                // creation, regions, and frame transmission.
                let frame = {
                    let size = handler.size.read().await;
                    let target_w = size.width;
                    let target_h = size.height;
                    drop(size);

                    if target_w > 0 && target_h > 0 {
                        let cropped = Self::crop_frame_to_size(&frame, target_w, target_h);
                        if cropped.width != frame.width || cropped.height != frame.height {
                            info!(
                                "📐 Cropped source frame {}×{} → client desktop {}×{} before encoding",
                                frame.width, frame.height, cropped.width, cropped.height
                            );
                        }
                        cropped
                    } else {
                        frame
                    }
                };

                let frame = if Self::should_flip_rdp_frame_vertical() {
                    Self::flip_frame_vertical(&frame)
                } else {
                    frame
                };

                let frame = if Self::should_rotate_rdp_frame_180() {
                    Self::rotate_frame_180(&frame)
                } else {
                    frame
                };

                let should_process = if adaptive_fps_enabled {
                    adaptive_fps.should_capture_frame()
                } else {
                    frame_regulator.should_send_frame()
                };

                if !should_process {
                    frames_dropped += 1;
                    if frames_dropped.is_multiple_of(30) {
                        let current_fps = if adaptive_fps_enabled {
                            adaptive_fps.current_fps()
                        } else {
                            legacy_fps
                        };
                        info!(
                            "Frame rate regulation: dropped {} frames, sent {}, target_fps={}",
                            frames_dropped, frames_sent, current_fps
                        );
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                    continue;
                }

                frames_sent += 1;
                if frames_sent.is_multiple_of(30) || frames_sent < 10 {
                    let activity = if adaptive_fps_enabled {
                        format!(
                            " [activity={:?}, fps={}]",
                            adaptive_fps.activity_level(),
                            adaptive_fps.current_fps()
                        )
                    } else {
                        String::new()
                    };
                    info!(
                        "🎬 Processing frame {} ({}x{}) - sent: {} (egfx: {}), dropped: {}{}",
                        frame.frame_id,
                        frame.width,
                        frame.height,
                        frames_sent,
                        egfx_frames_sent,
                        frames_dropped,
                        activity
                    );
                }

                // === WAIT FOR EGFX ===
                // Suppress output until EGFX is ready OR timeout expires.
                // Sending bitmap before EGFX establishes can cause display conflicts
                // when ResetGraphics clears the client's framebuffer. However, if EGFX
                // never becomes ready (no DVC, channel failure, etc.), we must fall
                // through to FastPath bitmap — otherwise the client gets zero frames.
                if !egfx_gate_bypassed && !handler.is_egfx_ready().await {
                    let since_first_frame = session_start.elapsed();
                    if first_frame_received && since_first_frame > egfx_timeout {
                        egfx_gate_bypassed = true;
                        warn!(
                            "EGFX not ready after {:.1}s, bypassing gate for FastPath bitmap delivery",
                            since_first_frame.as_secs_f64()
                        );
                    } else {
                        frames_dropped += 1;
                        if frames_dropped.is_multiple_of(30) {
                            let reason = handler.egfx_wait_reason().await;
                            debug!("⏳ {} (dropped {} frames)", reason, frames_dropped);
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                        continue;
                    }
                }

                // === EGFX LATE ARRIVAL ===
                // If EGFX was bypassed due to timeout but later becomes ready
                // (capability exchange completed after the 5s window), clear the
                // bypass so the Planar/AVC init path can run. Without this, the
                // replay loop spins forever: is_egfx_ready()=true + needs_init=true
                // but egfx_gate_bypassed=true prevents Planar init from executing.
                if egfx_gate_bypassed && handler.is_egfx_ready().await {
                    egfx_gate_bypassed = false;
                    info!(
                        "🔄 EGFX became ready after bypass — re-enabling EGFX path for Planar/AVC init"
                    );
                }

                // === EGFX/H.264 PATH ===
                // Only enter H.264 path when client supports AVC codec AND EGFX is
                // actually ready (not bypassed due to timeout). V8 clients (no AVC)
                // and clients where EGFX timed out skip this block entirely and fall
                // through to the FastPath bitmap path.
                //
                // Load egfx_needs_init but DON'T clear it yet for AVC clients.
                // If encoder or surface creation fails, we need the flag to stay
                // true so the next frame retries initialization. The flag is only
                // cleared on successful setup (egfx_sender populated).
                //
                // For V8 clients (no AVC), clear immediately since they never
                // enter the EGFX setup block and a stuck flag causes infinite
                // cached frame replay.
                let needs_init = if !egfx_gate_bypassed {
                    handler
                        .egfx_needs_init
                        .load(std::sync::atomic::Ordering::SeqCst)
                } else {
                    false
                };

                let negotiated_egfx_mode = if egfx_gate_bypassed {
                    None
                } else {
                    handler.negotiated_egfx_mode().await
                };
                let is_avc = negotiated_egfx_mode.is_some_and(NegotiatedEgfxMode::uses_avc);
                let use_bitmap = negotiated_egfx_mode == Some(NegotiatedEgfxMode::Bitmap);
                if use_bitmap {
                    egfx_gate_bypassed = true;
                    video_encoder = None;
                    egfx_sender = None;
                    handler
                        .egfx_needs_init
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                }
                if needs_init && !is_avc && !use_bitmap {
                    // Distinguish between:
                    // 1. V8 client (no EGFX channel at all) → clear flag now
                    // 2. EGFX negotiation still pending (e.g. Android V10 with AVC-disabled/Planar) → wait
                    //
                    // If gfx_handler_state exists but is_ready is false, the capability
                    // exchange hasn't completed yet. We must NOT clear egfx_needs_init
                    // because Planar setup runs after negotiation finishes.
                    let caps_pending = handler
                        .gfx_handler_state
                        .read()
                        .await
                        .as_ref()
                        .map(|s| !s.is_ready)
                        .unwrap_or(false);

                    if !caps_pending {
                        // V8 client: no EGFX capability state, no setup needed
                        handler
                            .egfx_needs_init
                            .store(false, std::sync::atomic::Ordering::SeqCst);
                    } else {
                        debug!("V8 check: skipping egfx_needs_init clear (caps still pending)");
                    }
                }

                if is_avc {
                    if needs_init {
                        // Reset encoder and sender for fresh client
                        // (This stale state belongs to an earlier client)
                        video_encoder = None;
                        egfx_sender = None;

                        // Invalidate damage detector to clear previous frame buffer
                        // This ensures first frame comparison returns 100% damage
                        if let Some(ref mut detector) = damage_detector_opt {
                            detector.invalidate();
                            info!("🔄 Damage detector invalidated for reconnection");
                        }

                        info!(
                            "🎬 EGFX channel ready - initializing H.264 encoder (needs_init=true)"
                        );

                        // AVC/H.264 path keeps encoder/surface dimensions 16-pixel
                        // aligned for Windows mstsc compatibility. Planar clients keep
                        // using actual-size surfaces in the separate Planar path above.
                        let display_width = frame.width as u16;
                        let display_height = frame.height as u16;
                        let encoded_width = align_to_16(frame.width) as u16;
                        let encoded_height = align_to_16(frame.height) as u16;

                        // Create H.264 encoder with resolution-appropriate level
                        // Use config values for quality settings and color space
                        let color_space = ColorSpaceConfig::from_config(
                            &self.config.egfx.color_matrix,
                            &self.config.egfx.color_range,
                            encoded_width as u32,
                            encoded_height as u32,
                        );
                        let config = EncoderConfig {
                            bitrate_kbps: self.config.egfx.h264_bitrate,
                            max_fps: self.config.video.target_fps as f32,
                            enable_skip_frame: true,
                            width: Some(encoded_width),
                            height: Some(encoded_height),
                            color_space: Some(color_space),
                            qp_min: self.config.egfx.qp_min,
                            qp_max: self.config.egfx.qp_max,
                            encoder_threads: self.config.performance.encoder_threads as u16,
                        };
                        h264_encoder_config = Some(config.clone());
                        let threads_desc = if self.config.performance.encoder_threads == 0 {
                            "auto".to_string()
                        } else {
                            self.config.performance.encoder_threads.to_string()
                        };
                        info!(
                            "🎬 H.264 encoder config: {}kbps, {}fps, QP[{}-{}], threads={}, color={}",
                            self.config.egfx.h264_bitrate,
                            self.config.video.target_fps,
                            self.config.egfx.qp_min,
                            self.config.egfx.qp_max,
                            threads_desc,
                            color_space.description()
                        );

                        // Determine codec based on config preference and client capabilities
                        // Config codec setting: "auto", "avc420", "avc444"
                        let negotiated_mode = handler.negotiated_egfx_mode().await;
                        let client_supports_avc444 =
                            negotiated_mode == Some(NegotiatedEgfxMode::Avc444);
                        if let Some(mode) = negotiated_mode {
                            info!("Client EGFX negotiated mode: {}", mode.name());
                        }

                        // Resolve codec preference from config
                        let codec_pref = self.config.egfx.codec.to_lowercase();
                        let avc444_enabled = match codec_pref.as_str() {
                            "avc420" => {
                                info!("Codec preference: AVC420 forced by config");
                                false
                            }
                            "avc444" => {
                                if client_supports_avc444 && self.config.egfx.avc444_enabled {
                                    info!("Codec preference: AVC444 requested and supported");
                                    true
                                } else if !client_supports_avc444 {
                                    info!(
                                        "Codec preference: AVC444 requested but client doesn't support it, using AVC420"
                                    );
                                    false
                                } else {
                                    info!(
                                        "Codec preference: AVC444 requested but disabled in config, using AVC420"
                                    );
                                    false
                                }
                            }
                            _ => {
                                // "auto" or unrecognized: use best available
                                if self.config.egfx.avc444_enabled && client_supports_avc444 {
                                    info!(
                                        "Codec preference: auto → AVC444 (client supports, enabled in config)"
                                    );
                                    true
                                } else if !self.config.egfx.avc444_enabled {
                                    info!(
                                        "Codec preference: auto → AVC420 (AVC444 disabled in config)"
                                    );
                                    false
                                } else {
                                    info!(
                                        "Codec preference: auto → AVC420 (client doesn't support AVC444)"
                                    );
                                    false
                                }
                            }
                        };

                        // ── Hardware encoder (VA-API) ──────────────────────
                        // Try hardware encoding first when enabled in config.
                        #[cfg(feature = "vaapi")]
                        {
                            if self.config.hardware_encoding.enabled
                                && !hardware_encoding_runtime_disabled
                            {
                                match create_hardware_encoder(
                                    &self.config.hardware_encoding,
                                    encoded_width as u32,
                                    encoded_height as u32,
                                ) {
                                    Ok(hw_encoder) => {
                                        video_encoder = Some(VideoEncoder::Hardware(
                                            SendHardwareEncoder(hw_encoder),
                                        ));
                                        info!(
                                            "✅ Hardware encoder initialized for {}×{} (GPU encoding)",
                                            encoded_width, encoded_height
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Hardware encoder failed: {:?} - falling back to OpenH264",
                                            e
                                        );
                                    }
                                }
                            }
                        }

                        // ── Software encoder (OpenH264) ──────────────────────────
                        // Only used when hardware encoder is unavailable or disabled.
                        if video_encoder.is_none() {
                            if avc444_enabled {
                                // Try AVC444 first (premium 4:4:4 chroma)
                                match Avc444Encoder::new(config.clone()) {
                                    Ok(mut encoder) => {
                                        // Wire aux omission config from EgfxConfig
                                        encoder.configure_aux_omission(
                                            self.config.egfx.avc444_enable_aux_omission,
                                            self.config.egfx.avc444_max_aux_interval,
                                            self.config.egfx.avc444_force_aux_idr_on_return,
                                        );
                                        // Wire periodic IDR config for artifact recovery
                                        encoder.configure_periodic_idr(
                                            self.config.egfx.periodic_idr_interval,
                                        );

                                        video_encoder = Some(VideoEncoder::Avc444(encoder));
                                        info!(
                                            "✅ AVC444 encoder initialized for {}×{} (4:4:4 chroma)",
                                            encoded_width, encoded_height
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to create AVC444 encoder: {:?} - falling back to AVC420",
                                            e
                                        );
                                        match Avc420Encoder::new(config) {
                                            Ok(encoder) => {
                                                video_encoder = Some(VideoEncoder::Avc420(encoder));
                                                info!(
                                                    "✅ AVC420 encoder initialized for {}×{} (4:2:0 fallback)",
                                                    encoded_width, encoded_height
                                                );
                                            }
                                            Err(e) => {
                                                warn!(
                                                    "Failed to create AVC420 encoder: {:?} - falling back to RemoteFX",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Use AVC420 (standard 4:2:0 chroma)
                                match Avc420Encoder::new(config) {
                                    Ok(encoder) => {
                                        video_encoder = Some(VideoEncoder::Avc420(encoder));
                                        info!(
                                            "✅ AVC420 encoder initialized for {}×{}",
                                            encoded_width, encoded_height
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to create H.264 encoder: {:?} - falling back to RemoteFX",
                                            e
                                        );
                                    }
                                }
                            }
                        }

                        // Only create EGFX surface when we have an encoder.
                        // Without an encoder, frames go via RemoteFX bitmaps and
                        // an orphan EGFX surface would put the client in mixed mode.
                        if video_encoder.is_none() {
                            info!(
                                "No H.264 encoder available, using RemoteFX bitmap path (no EGFX surface)"
                            );
                        } else if let (Some(gfx_handle), Some(event_tx)) = (
                            handler.gfx_server_handle.read().await.clone(),
                            handler.server_event_tx.read().await.clone(),
                        ) {
                            // Create primary surface for EGFX rendering.
                            // Must be done BEFORE sending any frames. For AVC,
                            // keep the RDP desktop/monitor at the visible size but
                            // create a 16-aligned surface for the encoded frame;
                            // the AVC region clips presentation to display_width/height.
                            {
                                info!(
                                    "📐 Creating EGFX AVC surface: display {}×{}, encoded surface {}×{}",
                                    display_width, display_height, encoded_width, encoded_height
                                );

                                let mut server =
                                    gfx_handle.lock().expect("GfxServerHandle mutex poisoned");

                                // CRITICAL: Set desktop size BEFORE creating surface.
                                // ResetGraphics advertises the visible desktop; CreateSurface
                                // may be larger for H.264 macroblock compatibility.
                                server.set_output_dimensions(display_width, display_height);
                                info!(
                                    "✅ EGFX desktop dimensions set: {}×{} (visible)",
                                    display_width, display_height
                                );

                                // Reset the EGFX output before creating the AVC surface.
                                crate::rdp::channels::graphics::egfx::channel::resize_with_primary_monitor(
                                    &mut server,
                                    display_width,
                                    display_height,
                                );

                                // Create the AVC surface at encoded dimensions. Windows mstsc
                                // requires H.264/AVC surfaces and bitstreams to stay 16-aligned;
                                // Planar clients use the separate actual-size path above.
                                if let Some(surface_id) =
                                    server.create_surface(encoded_width, encoded_height)
                                {
                                    if let Ok(mut shared) = handler.gfx_handler_state.try_write()
                                        && let Some(state) = shared.as_mut()
                                    {
                                        state.primary_surface_id = Some(surface_id);
                                    }
                                    info!(
                                        "✅ EGFX AVC surface {} created (encoded {}×{}, visible {}×{})",
                                        surface_id,
                                        encoded_width,
                                        encoded_height,
                                        display_width,
                                        display_height
                                    );
                                    // Map surface to output at origin (0,0)
                                    if server.map_surface_to_output(surface_id, 0, 0) {
                                        info!("✅ EGFX surface {} mapped to output", surface_id);
                                    } else {
                                        warn!("Failed to map EGFX surface to output");
                                    }
                                } else {
                                    warn!(
                                        "Failed to create EGFX surface - server may not be ready"
                                    );
                                }
                            }

                            let sender = EgfxChannelSender::new(
                                gfx_handle,
                                handler.gfx_handler_state.clone(),
                                event_tx,
                            );
                            match sender.flush_pending_server_messages().await {
                                Ok(count) if count > 0 => {
                                    info!("✅ EGFX surface messages sent to client");
                                }
                                Ok(_) => {}
                                Err(e) => warn!("EGFX: failed to flush surface messages: {e}"),
                            }
                            egfx_sender = Some(sender);
                            info!("✅ EGFX frame sender initialized");

                            // Setup succeeded: clear the init flag so we don't
                            // repeat encoder/surface creation on every frame
                            handler
                                .egfx_needs_init
                                .store(false, std::sync::atomic::Ordering::SeqCst);

                            // Force first frame to be sent regardless of damage detection
                            // This ensures reconnecting clients see the screen immediately
                            force_first_frame = true;
                            info!(
                                "📺 First frame after init will be forced (bypass damage detection)"
                            );
                        }
                    }

                    // Try to send via EGFX if encoder is available
                    if let (Some(encoder), Some(sender)) = (&mut video_encoder, &egfx_sender) {
                        // Use PipeWire PTS when available, fall back to synthetic timing
                        let timestamp_ms = if frame.pts > 0 {
                            frame.pts / 1_000_000 // nanoseconds → milliseconds
                        } else {
                            let frame_interval_ms =
                                1000 / u64::from(self.config.video.target_fps.max(1));
                            frames_sent * frame_interval_ms
                        };

                        // PipeWire sometimes sends zero-size buffers
                        let expected_size = (frame.width * frame.height * 4) as usize;
                        if frame.data.len() < expected_size {
                            trace!(
                                "Skipping invalid frame: size={}, expected={} for {}×{}",
                                frame.data.len(),
                                expected_size,
                                frame.width,
                                frame.height
                            );
                            frames_dropped += 1;
                            continue;
                        }

                        // === DAMAGE DETECTION (Config-controlled) ===
                        // Detect which regions changed since the last frame
                        // Skip encoding entirely if nothing changed (huge bandwidth savings)
                        //
                        // CRITICAL: Bypass damage detection when:
                        // 1. Periodic IDR is due (clear ghost artifacts)
                        // 2. First frame after initialization (reconnecting clients need immediate display)
                        let periodic_idr_due = encoder.is_periodic_idr_due();
                        let force_full_frame = periodic_idr_due || force_first_frame;

                        if force_first_frame {
                            info!("📺 Forcing first frame after init (IDR will be sent)");
                            force_first_frame = false;
                        }

                        let mut damage_regions = if force_full_frame {
                            // Force full frame - either periodic IDR or first frame after init
                            if periodic_idr_due {
                                debug!(
                                    "Forcing full frame for periodic IDR (bypassing damage detection)"
                                );
                            }
                            vec![DamageRegion::full_frame(frame.width, frame.height)]
                        } else if let Some(ref mut detector) = damage_detector_opt {
                            // Damage tracking enabled - detect changed regions
                            detector.detect(&frame.data, frame.width, frame.height)
                        } else {
                            // Damage tracking disabled - use full frame
                            vec![DamageRegion::full_frame(frame.width, frame.height)]
                        };

                        let damage_ratio = if !damage_regions.is_empty() {
                            let frame_area = (frame.width * frame.height) as u64;
                            let damage_area: u64 =
                                damage_regions.iter().map(DamageRegion::area).sum();
                            damage_area as f32 / frame_area as f32
                        } else {
                            0.0
                        };

                        if adaptive_fps_enabled {
                            adaptive_fps.update(damage_ratio);
                        }

                        let encoding_decision = latency_governor.should_encode_frame(damage_ratio);
                        match encoding_decision {
                            EncodingDecision::Skip => {
                                frames_dropped += 1;
                                continue;
                            }
                            EncodingDecision::WaitForMore => {
                                continue;
                            }
                            EncodingDecision::EncodeNow
                            | EncodingDecision::EncodeKeepalive
                            | EncodingDecision::EncodeBatch
                            | EncodingDecision::EncodeTimeout => {}
                        }

                        if damage_regions.is_empty() {
                            match encoding_decision {
                                EncodingDecision::EncodeKeepalive
                                | EncodingDecision::EncodeBatch
                                | EncodingDecision::EncodeTimeout => {
                                    // Do not consume a governor flush without sending a
                                    // frame. The accumulated damage may come from an
                                    // earlier capture, so refresh the complete surface.
                                    damage_regions =
                                        vec![DamageRegion::full_frame(frame.width, frame.height)];
                                }
                                EncodingDecision::EncodeNow
                                | EncodingDecision::Skip
                                | EncodingDecision::WaitForMore => {
                                    frames_skipped_damage += 1;
                                    if frames_skipped_damage.is_multiple_of(100)
                                        && let Some(ref detector) = damage_detector_opt
                                    {
                                        let stats = detector.stats();
                                        debug!(
                                            "🎯 Damage tracking: {} frames skipped (no change), {:.1}% bandwidth saved",
                                            frames_skipped_damage,
                                            stats.bandwidth_reduction_percent()
                                        );
                                    }
                                    continue;
                                }
                            }
                        }

                        if frames_sent.is_multiple_of(60) {
                            if let Some(ref detector) = damage_detector_opt {
                                let stats = detector.stats();
                                debug!(
                                    "🎯 Damage: {} regions, {:.1}% of frame, avg {:.1}ms detection",
                                    damage_regions.len(),
                                    damage_ratio * 100.0,
                                    stats.avg_detection_time_ms
                                );
                            }
                            if adaptive_fps_enabled {
                                debug!(
                                    "🎛️ Adaptive FPS: activity={:?}, fps={}, latency_mode={:?}",
                                    adaptive_fps.activity_level(),
                                    adaptive_fps.current_fps(),
                                    latency_governor.mode()
                                );
                            }
                        }

                        // AVC/H.264 uses 16-aligned encoded dimensions for Windows
                        // compatibility. Keep display_width/display_height as the visible
                        // region, and pad only the encoder input so Planar/Android remains
                        // actual-size in its separate path.
                        let encoded_width = align_to_16(frame.width);
                        let encoded_height = align_to_16(frame.height);
                        let frame_data =
                            if encoded_width != frame.width || encoded_height != frame.height {
                                Cow::Owned(Self::pad_frame_to_aligned(
                                    &frame.data,
                                    frame.width,
                                    frame.height,
                                    encoded_width,
                                    encoded_height,
                                ))
                            } else {
                                Cow::Borrowed(frame.data.as_slice())
                            };

                        // OpenH264's encode() is synchronous and CPU-bound.
                        // On slow hardware (e.g., QEMU VMs) it can block for seconds.
                        // block_in_place tells tokio this thread is occupied so the
                        // runtime can schedule other tasks on remaining threads.
                        let encode_result = tokio::task::block_in_place(|| {
                            encoder.encode_bgra(
                                frame_data.as_ref(),
                                encoded_width,
                                encoded_height,
                                timestamp_ms,
                            )
                        });
                        match encode_result {
                            Ok(Some(encoded_frame)) => {
                                let codec_name = encoder.codec_name();
                                let payload_len = encoded_frame.payload_len();
                                if let Some(reason) =
                                    hardware_runtime_fallback_reason(codec_name, Some(payload_len))
                                {
                                    warn!(
                                        "{} from {}; disabling hardware encoding for this session and falling back to OpenH264 AVC420",
                                        reason, codec_name
                                    );
                                    #[cfg(feature = "vaapi")]
                                    {
                                        hardware_encoding_runtime_disabled = true;
                                    }
                                    if let Some(config) = h264_encoder_config.clone() {
                                        match Avc420Encoder::new(config) {
                                            Ok(sw_encoder) => {
                                                *encoder = VideoEncoder::Avc420(sw_encoder);
                                                force_first_frame = true;
                                            }
                                            Err(e) => {
                                                error!(
                                                    "Failed to create OpenH264 fallback encoder after hardware runtime failure: {:?}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    frames_dropped += 1;
                                    continue;
                                }

                                let send_result = match encoded_frame {
                                    EncodedVideoFrame::Single(data) => {
                                        sender
                                            .send_frame_with_regions(
                                                &data,
                                                encoded_width as u16,
                                                encoded_height as u16,
                                                frame.width as u16,
                                                frame.height as u16,
                                                &damage_regions,
                                                timestamp_ms as u32,
                                            )
                                            .await
                                    }
                                    EncodedVideoFrame::Dual { main, aux } => {
                                        sender
                                            .send_avc444_frame_with_regions(
                                                &main,
                                                aux.as_deref(), // Option<Vec<u8>> → Option<&[u8]>
                                                encoded_width as u16,
                                                encoded_height as u16,
                                                frame.width as u16,
                                                frame.height as u16,
                                                &damage_regions,
                                                timestamp_ms as u32,
                                            )
                                            .await
                                    }
                                };

                                match send_result {
                                    Ok(_frame_id) => {
                                        egfx_frames_sent += 1;
                                        if egfx_frames_sent.is_multiple_of(30) {
                                            let codec = encoder.codec_name();
                                            debug!(
                                                "📹 EGFX: Sent {} {} frames",
                                                egfx_frames_sent, codec
                                            );
                                        }
                                        continue; // Frame sent via EGFX, skip RemoteFX path
                                    }
                                    Err(e) => {
                                        if encoder.is_hardware() {
                                            warn!(
                                                "EGFX send failed after hardware encode: {}; disabling hardware encoding for this session and falling back to OpenH264 AVC420",
                                                e
                                            );
                                            #[cfg(feature = "vaapi")]
                                            {
                                                hardware_encoding_runtime_disabled = true;
                                            }
                                            if let Some(config) = h264_encoder_config.clone() {
                                                match Avc420Encoder::new(config) {
                                                    Ok(sw_encoder) => {
                                                        *encoder = VideoEncoder::Avc420(sw_encoder);
                                                        force_first_frame = true;
                                                    }
                                                    Err(err) => {
                                                        error!(
                                                            "Failed to create OpenH264 fallback encoder after EGFX send failure: {:?}",
                                                            err
                                                        );
                                                    }
                                                }
                                            }
                                        } else {
                                            // CRITICAL: Once EGFX is active, NEVER fall back to RemoteFX!
                                            // Mixing codecs causes display conflicts - EGFX surface invisible
                                            trace!(
                                                "EGFX send failed: {} - dropping frame (no RemoteFX fallback)",
                                                e
                                            );
                                        }
                                        frames_dropped += 1;
                                        continue; // Drop frame, don't fall through to RemoteFX
                                    }
                                }
                            }
                            Ok(None) => {
                                let codec_name = encoder.codec_name();
                                if let Some(reason) =
                                    hardware_runtime_fallback_reason(codec_name, None)
                                {
                                    warn!(
                                        "{} from {}; disabling hardware encoding for this session and falling back to OpenH264 AVC420",
                                        reason, codec_name
                                    );
                                    #[cfg(feature = "vaapi")]
                                    {
                                        hardware_encoding_runtime_disabled = true;
                                    }
                                    if let Some(config) = h264_encoder_config.clone() {
                                        match Avc420Encoder::new(config) {
                                            Ok(sw_encoder) => {
                                                *encoder = VideoEncoder::Avc420(sw_encoder);
                                                force_first_frame = true;
                                            }
                                            Err(e) => {
                                                error!(
                                                    "Failed to create OpenH264 fallback encoder after hardware no-output: {:?}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                } else {
                                    trace!("H.264 encoder skipped frame");
                                }
                                frames_dropped += 1;
                                continue;
                            }
                            Err(e) => {
                                if encoder.is_hardware() {
                                    warn!(
                                        "Hardware H.264 encoding failed: {:?}; disabling hardware encoding for this session and falling back to OpenH264 AVC420",
                                        e
                                    );
                                    #[cfg(feature = "vaapi")]
                                    {
                                        hardware_encoding_runtime_disabled = true;
                                    }
                                    if let Some(config) = h264_encoder_config.clone() {
                                        match Avc420Encoder::new(config) {
                                            Ok(sw_encoder) => {
                                                *encoder = VideoEncoder::Avc420(sw_encoder);
                                                force_first_frame = true;
                                            }
                                            Err(err) => {
                                                error!(
                                                    "Failed to create OpenH264 fallback encoder after hardware encode error: {:?}",
                                                    err
                                                );
                                            }
                                        }
                                    }
                                } else {
                                    // CRITICAL: Once EGFX is active, don't fall back to RemoteFX
                                    trace!(
                                        "H.264 encoding failed: {:?} - dropping frame (no RemoteFX fallback)",
                                        e
                                    );
                                }
                                frames_dropped += 1;
                                continue; // Drop frame, don't fall through to RemoteFX
                            }
                        }
                    }
                }

                let convert_start = std::time::Instant::now();
                let target_size = *handler.size.read().await;
                let bitmap_frame =
                    Self::crop_frame_to_size(&frame, target_size.width, target_size.height);
                if bitmap_frame.width != frame.width || bitmap_frame.height != frame.height {
                    debug!(
                        "Cropping bitmap fallback frame: {}x{} -> {}x{}",
                        frame.width, frame.height, bitmap_frame.width, bitmap_frame.height
                    );
                }
                // BGRx passthrough: PipeWire always produces BGRx, and when the
                // client desktop also uses BGRx the BitmapConverter's generic path
                // rejects BGRx→BGRx as unsupported. Build the BitmapUpdate directly
                // from the compact frame data to bypass the converter.
                use crate::desktop::pipewire::PixelFormat as PwPixelFormat;
                let bitmap_update = if bitmap_frame.format == PwPixelFormat::BGRx {
                    let bpp = 4usize;
                    let expected_len =
                        bitmap_frame.width as usize * bitmap_frame.height as usize * bpp;
                    if bitmap_frame.data.len() < expected_len {
                        error!(
                            "BGRx passthrough: frame too small: len={} expected={} for {}×{}",
                            bitmap_frame.data.len(),
                            expected_len,
                            bitmap_frame.width,
                            bitmap_frame.height
                        );
                        frames_dropped += 1;
                        continue;
                    }
                    BitmapUpdate {
                        rectangles: vec![BitmapData {
                            rectangle: Rectangle::new(
                                0,
                                0,
                                bitmap_frame.width as u16,
                                bitmap_frame.height as u16,
                            ),
                            format: RdpPixelFormat::BgrX32,
                            data: bitmap_frame.data[..expected_len].to_vec(),
                        }],
                    }
                } else {
                    match handler.convert_to_bitmap(bitmap_frame).await {
                        Ok(bitmap) => bitmap,
                        Err(e) => {
                            error!("Failed to convert frame to bitmap: {}", e);
                            continue;
                        }
                    }
                };
                let convert_elapsed = convert_start.elapsed();

                // EARLY EXIT: Skip empty frames BEFORE expensive IronRDP conversion
                // BitmapConverter returns empty rectangles when frame unchanged (dirty region optimization)
                // This saves ~1-2ms per unchanged frame (40% of frames!)
                if bitmap_update.rectangles.is_empty() {
                    // Log periodically to verify optimization is working
                    static EMPTY_COUNT: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(0);
                    let count = EMPTY_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if count.is_multiple_of(100) && count > 0 {
                        debug!(
                            "Empty frame optimization: {} unchanged frames skipped",
                            count
                        );
                    }
                    continue;
                }

                let iron_start = std::time::Instant::now();
                let iron_updates = match handler.convert_to_iron_format(&bitmap_update).await {
                    Ok(updates) => updates,
                    Err(e) => {
                        error!("Failed to convert to IronRDP format: {}", e);
                        continue;
                    }
                };
                let iron_elapsed = iron_start.elapsed();

                if frames_sent.is_multiple_of(30) {
                    info!(
                        "🎨 Frame conversion timing: bitmap={:?}, iron={:?}, total={:?}",
                        convert_elapsed,
                        iron_elapsed,
                        convert_start.elapsed()
                    );
                }

                if let Some(ref graphics_tx) = handler.graphics_tx {
                    for iron_bitmap in iron_updates {
                        let graphics_frame = GraphicsFrame::new(iron_bitmap, frames_sent);

                        trace!(
                            "📤 Graphics multiplexer: sending frame {} to queue",
                            frames_sent
                        );
                        if let Err(_e) = graphics_tx.try_send(graphics_frame) {
                            warn!("Graphics queue full - frame dropped (QoS policy)");
                        }
                    }
                } else {
                    let sender = handler.update_sender.lock().await;
                    for iron_bitmap in iron_updates {
                        let update = DisplayUpdate::Bitmap(iron_bitmap);

                        if let Err(e) = sender.send(update).await {
                            error!("Failed to send display update: {}", e);
                            return;
                        }
                    }
                }
            }
        });
    }

    /// Convert video frame to RDP bitmap
    async fn convert_to_bitmap(&self, frame: VideoFrame) -> Result<BitmapUpdate> {
        let mut converter = self.bitmap_converter.lock().await;
        converter
            .convert_frame(&frame)
            .map_err(|e| anyhow::anyhow!("Bitmap conversion failed: {e}"))
    }

    /// Convert our BitmapUpdate format to IronRDP's BitmapUpdate format
    async fn convert_to_iron_format(&self, update: &BitmapUpdate) -> Result<Vec<IronBitmapUpdate>> {
        let mut iron_updates = Vec::new();

        for rect_data in &update.rectangles {
            let iron_format = match rect_data.format {
                RdpPixelFormat::BgrX32 => IronPixelFormat::BgrX32,
                RdpPixelFormat::Bgr24 => {
                    // IronRDP doesn't have Bgr24, use XBgr32 instead.
                    warn!("Converting Bgr24 to XBgr32 for IronRDP compatibility");
                    IronPixelFormat::XBgr32
                }
            };

            let width = rect_data
                .rectangle
                .right
                .saturating_sub(rect_data.rectangle.left);
            let height = rect_data
                .rectangle
                .bottom
                .saturating_sub(rect_data.rectangle.top);

            let bytes_per_pixel = iron_format.bytes_per_pixel() as usize;
            let stride = NonZeroUsize::new(width as usize * bytes_per_pixel)
                .ok_or_else(|| anyhow::anyhow!("Invalid stride calculation: width={width}"))?;

            let iron_bitmap = IronBitmapUpdate {
                x: rect_data.rectangle.left,
                y: rect_data.rectangle.top,
                width: NonZeroU16::new(width)
                    .ok_or_else(|| anyhow::anyhow!("Invalid width: {width}"))?,
                height: NonZeroU16::new(height)
                    .ok_or_else(|| anyhow::anyhow!("Invalid height: {height}"))?,
                format: iron_format,
                data: Bytes::from(rect_data.data.clone()),
                stride,
            };

            if !LOGGED_FIRST_BITMAP_UPDATE.swap(true, Ordering::Relaxed) {
                info!(
                    "First IronRDP bitmap update: x={} y={} width={} height={} format={:?} stride={} data_len={} expected_compact_len={}",
                    iron_bitmap.x,
                    iron_bitmap.y,
                    iron_bitmap.width.get(),
                    iron_bitmap.height.get(),
                    iron_bitmap.format,
                    iron_bitmap.stride.get(),
                    iron_bitmap.data.len(),
                    width as usize * height as usize * bytes_per_pixel,
                );
            }

            iron_updates.push(iron_bitmap);
        }

        Ok(iron_updates)
    }
}

#[async_trait::async_trait]
impl RdpServerDisplay for DisplayChannelHandler {
    async fn size(&mut self) -> DesktopSize {
        let size = self.size.read().await;
        *size
    }

    async fn request_initial_size(&mut self, client_size: DesktopSize) -> DesktopSize {
        let mut size = self.size.write().await;
        info!(
            "request_initial_size: client requested {}x{}, current server size {}x{}",
            client_size.width, client_size.height, size.width, size.height
        );

        if let Some((width, height)) =
            self.allowed_resize(u32::from(client_size.width), u32::from(client_size.height))
        {
            let accepted_size = DesktopSize { width, height };
            let size_changed = *size != accepted_size;
            if size_changed {
                if let Err(e) = self.resize_managed_compositor(width, height).await {
                    warn!(
                        "Failed to apply initial compositor size; keeping server desktop size: {e}"
                    );
                    return *size;
                }

                // Let screencopy receive the compositor mode change and allocate
                // a correctly-sized buffer before initial graphics negotiation completes.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            info!(
                "Accepting client initial desktop size: {}x{} (was {}x{})",
                width, height, size.width, size.height
            );
            *size = accepted_size;
            *self.bitmap_converter.lock().await = BitmapConverter::new(width, height);
        } else {
            debug!(
                "Keeping server initial desktop size: {}x{} (client requested {}x{})",
                size.width, size.height, client_size.width, client_size.height
            );
        }
        *size
    }

    /// Called once per connection to establish the update stream.
    /// If an earlier connection consumed the receiver, we create a fresh channel
    /// to allow reconnection without requiring server restart.
    #[expect(
        clippy::expect_used,
        reason = "mutex poisoning is unrecoverable; receiver guaranteed after reset"
    )]
    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        let mut receiver_option = self.update_receiver.lock().await;

        // If receiver was already taken by an earlier connection, create a new channel
        if receiver_option.is_none() {
            debug!("Display updates channel exhausted, creating new channel for reconnection");
            let (new_sender, new_receiver) = mpsc::channel(64);
            *self.update_sender.lock().await = new_sender;
            *receiver_option = Some(new_receiver);

            // CRITICAL: Reset ALL EGFX state for new client
            // The new client needs fresh EGFX negotiation + ResetGraphics + CreateSurface.
            // Without these resets:
            // 1. egfx_needs_init=false would skip encoder/surface creation
            // 2. stale gfx_handler_state.is_ready=true would skip waiting for new EGFX channel
            // 3. stale gfx_server_handle would have old surface (create_surface returns None)
            info!("Resetting EGFX state for reconnecting client");
            self.egfx_needs_init
                .store(true, std::sync::atomic::Ordering::SeqCst);

            // Do not clear gfx_handler_state/gfx_server_handle here. IronRDP calls
            // EgfxChannelFactory::build_server_with_handle() while attaching channels
            // for the new connection; that factory installs the fresh handle and
            // clears readiness before capability negotiation. Clearing after that
            // point races with Android EGFX AVC-disabled/Planar negotiation and leaves the
            // display pipeline stuck in FastPath bitmap fallback (mouse works,
            // video black).

            // Reset bitmap converter so the new client gets a full initial frame.
            // The converter caches the last frame hash for dirty-region optimization;
            // without this reset, the replayed cached frame matches the hash and
            // produces an empty update (zero visible bitmap data).
            //
            // Use try_lock to avoid potential deadlock with the pipeline loop.
            // If the lock isn't available, force_full_update will be called when
            // the pipeline processes the next frame.
            match self.bitmap_converter.try_lock() {
                Ok(mut converter) => {
                    let size = self.size.read().await;
                    *converter = BitmapConverter::new(size.width, size.height);
                    debug!("Reset BitmapConverter for {}x{}", size.width, size.height);
                }
                _ => {
                    debug!("BitmapConverter locked by pipeline, will reset on next frame");
                }
            }

            // Notify input handler about reconnection
            // The input handler is shared across connections but needs to reset internal state
            // (keyboard modifiers, mouse button state) when a new client connects
            if let Some(ref handler) = *self.input_handler.read().await {
                handler.notify_reconnection().await;
            }

            // On reconnect, the clipboard provider manages its own state cleanup.
            if self.clipboard_manager.read().await.is_some() {
                info!("Reconnection detected - clipboard provider handles state reset");
            }
        }

        // Signal pipeline that a client is now consuming frames
        self.client_active
            .store(true, std::sync::atomic::Ordering::SeqCst);
        info!("Client active - pipeline frame processing resumed");

        // Use the client's native/default pointer for normal clients. The
        // RGBAPointer helper below is vertically flipped for the Android RD
        // Client workaround and has an Android-specific hotspot; sending it to
        // every client makes the visible pointer tip differ from the click point.
        // Android clients that actually need a bitmap pointer still receive one
        // from the input handler after EGFX negotiation marks that quirk.
        {
            let sender = self.update_sender.lock().await;
            if let Err(err) = sender.try_send(DisplayUpdate::DefaultPointer) {
                trace!("Dropping initial default pointer update: {err}");
            }
        }

        let receiver = receiver_option
            .take()
            .expect("receiver should exist after reset");

        Ok(Box::new(DisplayUpdatesStream::new(receiver)))
    }

    fn request_layout(&mut self, layout: ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout) {
        let monitors = layout.monitors();
        info!(
            "Client requested layout change: {} monitor(s)",
            monitors.len()
        );

        info!(
            "Client layout change received in direct-channel mode; will try temporary compositor output resize"
        );

        // Extract the primary monitor (or first monitor for single-monitor case)
        let monitor = match monitors.iter().find(|m| m.is_primary()) {
            Some(m) => m,
            None => match monitors.first() {
                Some(m) => m,
                None => {
                    warn!("Empty monitor layout received, ignoring");
                    return;
                }
            },
        };

        let (raw_w, raw_h) = monitor.dimensions();

        let Some((new_w, new_h)) = self.allowed_resize(raw_w, raw_h) else {
            return;
        };

        // Skip if same as current size
        if let Ok(current) = self.size.try_read()
            && current.width == new_w
            && current.height == new_h
        {
            debug!("Requested resolution matches current, ignoring");
            return;
        }

        info!(
            "Resize request accepted: {}x{} (raw: {}x{})",
            new_w, new_h, raw_w, raw_h
        );

        // Replace rather than enqueue: the pipeline only needs the final size
        // from a burst of display-control PDUs.
        match self.pending_resize.lock() {
            Ok(mut pending) => {
                *pending = Some(ResizeRequest {
                    width: new_w,
                    height: new_h,
                });
                debug!("Latest resize request stored for pipeline");
            }
            Err(error) => error!("Resize request lock poisoned: {error}"),
        }
    }
}

/// Clone implementation for WrdDisplayHandler
///
/// Allows the handler to be cloned for use with IronRDP's builder pattern.
/// All internal state is Arc'd so cloning is cheap and maintains shared state.
impl Clone for DisplayChannelHandler {
    fn clone(&self) -> Self {
        Self {
            size: Arc::clone(&self.size),
            pipewire_thread: Arc::clone(&self.pipewire_thread),
            bitmap_converter: Arc::clone(&self.bitmap_converter),
            update_sender: Arc::clone(&self.update_sender),
            update_receiver: Arc::clone(&self.update_receiver),
            graphics_tx: self.graphics_tx.clone(),
            stream_info: self.stream_info.clone(),
            compositor_socket: self.compositor_socket.clone(),
            // EGFX fields
            gfx_server_handle: Arc::clone(&self.gfx_server_handle),
            gfx_handler_state: Arc::clone(&self.gfx_handler_state),
            server_event_tx: Arc::clone(&self.server_event_tx),
            config: Arc::clone(&self.config), // Clone config Arc
            service_registry: Arc::clone(&self.service_registry), // Clone service registry Arc
            egfx_needs_init: Arc::clone(&self.egfx_needs_init), // Share EGFX init state
            input_handler: Arc::clone(&self.input_handler), // Share input handler ref
            clipboard_manager: Arc::clone(&self.clipboard_manager), // Share clipboard manager ref
            pending_resize: Arc::clone(&self.pending_resize),
            client_active: Arc::clone(&self.client_active),
            health_reporter: Arc::clone(&self.health_reporter),
        }
    }
}

struct DisplayUpdatesStream {
    receiver: mpsc::Receiver<DisplayUpdate>,
}

impl DisplayUpdatesStream {
    fn new(receiver: mpsc::Receiver<DisplayUpdate>) -> Self {
        Self { receiver }
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for DisplayUpdatesStream {
    /// Cancellation-safe as required by IronRDP.
    async fn next_update(&mut self) -> Result<Option<DisplayUpdate>> {
        match self.receiver.recv().await {
            Some(update) => {
                trace!("Providing display update");
                Ok(Some(update))
            }
            None => {
                debug!("Display update stream closed");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdp::channels::graphics::bitmap::converter::{BitmapData, Rectangle};

    #[tokio::test]
    async fn test_pixel_format_conversion() {
        // Test our format conversion logic
        let formats = vec![
            (RdpPixelFormat::BgrX32, IronPixelFormat::BgrX32),
            (RdpPixelFormat::Bgr24, IronPixelFormat::XBgr32),
        ];

        for (our_format, iron_format) in formats {
            // Verify bytes_per_pixel matches the reduced internal bitmap surface.
            let our_bpp = our_format.bytes_per_pixel();
            // IronRDP formats are all 32-bit
            let iron_bpp = iron_format.bytes_per_pixel();
            debug!(
                "Format {:?} -> {:?}: {} bpp -> {} bpp",
                our_format, iron_format, our_bpp, iron_bpp
            );
        }
    }

    #[test]
    fn compacting_frame_removes_source_stride_padding() {
        let frame = VideoFrame {
            frame_id: 1,
            pts: 0,
            dts: 0,
            duration: 0,
            width: 2,
            height: 2,
            stride: 12,
            format: crate::desktop::pipewire::PixelFormat::BGRx,
            monitor_index: 0,
            data: Arc::new(vec![
                1, 2, 3, 4, 5, 6, 7, 8, 90, 91, 92, 93, 9, 10, 11, 12, 13, 14, 15, 16, 94, 95, 96,
                97,
            ]),
            capture_time: std::time::SystemTime::now(),
            damage_regions: vec![],
            flags: crate::desktop::pipewire::frame::FrameFlags::new(),
        };

        let compact = DisplayChannelHandler::crop_frame_to_size(&frame, 2, 2);

        assert_eq!(compact.stride, 8);
        assert_eq!(
            compact.data.as_slice(),
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn hardware_runtime_failures_trigger_software_fallback() {
        assert_eq!(
            hardware_runtime_fallback_reason("VA-API H.264", None),
            Some("hardware encoder returned no frame")
        );
        assert_eq!(
            hardware_runtime_fallback_reason("VA-API H.264", Some(0)),
            Some("hardware encoder produced empty H.264 payload")
        );
        assert_eq!(
            hardware_runtime_fallback_reason("AVC420", Some(0)),
            None,
            "software encoder failures must not be misclassified as hardware fallback"
        );
        assert_eq!(
            hardware_runtime_fallback_reason("VA-API H.264", Some(128)),
            None
        );
    }

    #[tokio::test]
    async fn test_bitmap_data_structure() {
        // Verify our understanding of BitmapData structure
        let rect = Rectangle::new(0, 0, 100, 100);
        let data = BitmapData {
            rectangle: rect,
            format: RdpPixelFormat::BgrX32,
            data: vec![0u8; 100 * 100 * 4],
        };

        assert_eq!(data.rectangle.left, 0);
        assert_eq!(data.rectangle.top, 0);
        assert_eq!(data.rectangle.right, 100);
        assert_eq!(data.rectangle.bottom, 100);
        assert_eq!(data.data.len(), 100 * 100 * 4);
    }
}
