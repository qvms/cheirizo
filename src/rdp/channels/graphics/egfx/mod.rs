//! EGFX graphics pipeline integration for wrdp runtime.
//!
//! This module groups the pieces that turn captured desktop frames into RDP
//! graphics pipeline updates: frame handlers, software and optional hardware
//! encoders, color conversion policy, and AVC444 packing. The exported types are
//! the narrow surface used by the server layer; most encoder implementation
//! details stay private to the EGFX subsystem.
//!
//! Hardware encoder modules remain feature-gated, so builds without VA-API continue to use the software/OpenH264 path only.

pub mod channel;
pub(crate) mod color;
pub(crate) mod h264;

#[cfg(feature = "vaapi")]
pub mod hardware;

mod handler;

pub use color::convert::{ColorMatrix, Yuv444Frame, bgra_to_yuv444, subsample_chroma_420};
pub use color::space::{
    ColorRange, ColorSpaceConfig, ColourPrimaries, MatrixCoefficients, TransferCharacteristics,
};
pub use color::yuv444_packing::{
    Yuv420Frame, pack_auxiliary_view, pack_dual_views, pack_main_view, validate_dimensions,
};
pub use h264::avc444::encoder::{Avc444Encoder, Avc444Frame, Avc444Stats, Avc444Timing};
pub use h264::encoder::{
    Avc420Encoder, EncoderConfig, EncoderError, EncoderResult, EncoderStats, H264Frame, align_to_16,
};
pub use h264::level::{ConstraintViolation, H264Level, LevelConstraints};
// `WrdpGraphicsHandler` implements the EGFX pipeline handler trait internally,
// but that trait is intentionally not part of this module's public API.
pub use handler::WrdpGraphicsHandler;
#[cfg(feature = "vaapi")]
pub use hardware::{
    HardwareEncoder, HardwareEncoderError, HardwareEncoderResult, HardwareEncoderStats,
    QualityPreset, create_hardware_encoder,
};

// IronRDP EGFX protocol types (Avc420Region, GraphicsPipelineServer, etc.)
// are not re-exported here; they remain internal to `channel`.
