//! Hardware encoder error taxonomy.
//!
//! GPU backends report detailed backend failures, but callers usually only need
//! to decide whether hardware is unavailable, an encode can be retried, or a
//! configuration/runtime error should trigger software fallback. This module
//! keeps those policy decisions local to the EGFX hardware layer.

use std::path::PathBuf;

use thiserror::Error;

/// Broad handling class for a hardware-encoder failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareErrorClass {
    BackendUnavailable,
    RetryableRuntime,
    FatalRuntime,
    Configuration,
    BackendDiagnostic,
}

/// Unified error type for hardware encoding operations.
#[derive(Debug, Error)]
pub enum HardwareEncoderError {
    /// No hardware encoding backend is available on this system.
    #[error("No hardware encoder available: {reason}")]
    NoBackendAvailable { reason: String },

    /// The specified GPU device could not be opened.
    #[error("Device not found: {path}")]
    DeviceNotFound { path: PathBuf },

    /// The GPU does not support H.264 encoding.
    #[error("H.264 encoding not supported by hardware")]
    H264NotSupported,

    /// Failed to initialize the encoder context.
    #[error("Encoder initialization failed: {0}")]
    InitFailed(String),

    /// The requested encoder configuration is not supported.
    #[error("Unsupported configuration: {0}")]
    UnsupportedConfig(String),

    /// Frame encoding failed.
    #[error("Encode failed: {0}")]
    EncodeFailed(String),

    /// No more surfaces/buffers available in the pool.
    #[error("Buffer pool exhausted (need {needed}, have {available})")]
    BufferPoolExhausted { needed: usize, available: usize },

    /// Invalid frame dimensions provided.
    #[error("Invalid dimensions: {width}x{height} - {reason}")]
    InvalidDimensions {
        width: u32,
        height: u32,
        reason: String,
    },

    /// Color conversion failed (BGRA to NV12).
    #[error("Color conversion failed: {0}")]
    ColorConversionFailed(String),

    /// Timeout waiting for encoder to complete.
    #[error("Encoder timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// Dynamic resolution change is not supported by this backend.
    #[error("Resolution reconfiguration not supported by {backend}")]
    ReconfigureNotSupported { backend: &'static str },

    /// Invalid quality preset specified.
    #[error("Invalid quality preset: {preset} (valid: speed, balanced, quality)")]
    InvalidPreset { preset: String },

    /// VA-API specific error.
    #[cfg(feature = "vaapi")]
    #[error("VA-API error: {0}")]
    Vaapi(#[from] VaapiError),
}

impl HardwareEncoderError {
    pub fn class(&self) -> HardwareErrorClass {
        match self {
            Self::NoBackendAvailable { .. }
            | Self::DeviceNotFound { .. }
            | Self::H264NotSupported
            | Self::InitFailed(_) => HardwareErrorClass::BackendUnavailable,
            Self::BufferPoolExhausted { .. } | Self::Timeout { .. } => {
                HardwareErrorClass::RetryableRuntime
            }
            Self::UnsupportedConfig(_)
            | Self::InvalidDimensions { .. }
            | Self::ReconfigureNotSupported { .. }
            | Self::InvalidPreset { .. } => HardwareErrorClass::Configuration,
            Self::EncodeFailed(_) | Self::ColorConversionFailed(_) => {
                HardwareErrorClass::FatalRuntime
            }
            #[cfg(feature = "vaapi")]
            Self::Vaapi(_) => HardwareErrorClass::BackendDiagnostic,
        }
    }

    pub fn is_backend_unavailable(&self) -> bool {
        self.class() == HardwareErrorClass::BackendUnavailable
    }

    pub fn is_recoverable(&self) -> bool {
        self.class() == HardwareErrorClass::RetryableRuntime
    }
}

/// VA-API backend specific errors.
#[cfg(feature = "vaapi")]
#[derive(Debug, Error)]
pub enum VaapiError {
    /// Failed to open DRM render device.
    #[error("Failed to open device {path}: {source}")]
    DeviceOpenFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to initialize VA display.
    #[error("VA display initialization failed: {0}")]
    DisplayInitFailed(String),

    /// VA-API version is below the minimum supported level.
    #[error("VA-API version {major}.{minor} too old (need {min_major}.{min_minor}+)")]
    VersionTooOld {
        major: i32,
        minor: i32,
        min_major: i32,
        min_minor: i32,
    },

    /// Profile query failed.
    #[error("Failed to query VA profiles: {0}")]
    ProfileQueryFailed(String),

    /// Entrypoint query failed.
    #[error("Failed to query entrypoints: {0}")]
    EntrypointQueryFailed(String),

    /// H.264 encoding not supported.
    #[error("H.264 encoding not supported by this GPU")]
    H264NotSupported,

    /// Encode entrypoint not available.
    #[error("Encode entrypoint not available")]
    EncodeNotSupported,

    /// Config creation failed.
    #[error("Config creation failed: {0}")]
    ConfigCreateFailed(String),

    /// Surface creation failed.
    #[error("Surface creation failed: {0}")]
    SurfaceCreateFailed(String),

    /// Context creation failed.
    #[error("Context creation failed: {0}")]
    ContextCreateFailed(String),

    /// VPP (Video Post-Processing) not available.
    #[error("VPP not available for color conversion")]
    VppNotAvailable,

    /// Buffer operation failed.
    #[error("Buffer operation failed: {0}")]
    BufferError(String),

    /// VA operation returned error status.
    #[error("VA call failed: {function}() returned {status}")]
    VaCallFailed { function: &'static str, status: i32 },

    /// Sync operation failed or timed out.
    #[error("Surface sync failed: {0}")]
    SyncFailed(String),
}

pub type HardwareEncoderResult<T> = Result<T, HardwareEncoderError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_backend_availability_failures() {
        let cases = [
            HardwareEncoderError::NoBackendAvailable {
                reason: "missing driver".into(),
            },
            HardwareEncoderError::DeviceNotFound {
                path: PathBuf::from("/dev/dri/renderD128"),
            },
            HardwareEncoderError::H264NotSupported,
            HardwareEncoderError::InitFailed("no display".into()),
        ];

        for err in cases {
            assert_eq!(err.class(), HardwareErrorClass::BackendUnavailable);
            assert!(err.is_backend_unavailable());
            assert!(!err.is_recoverable());
        }
    }

    #[test]
    fn classifies_retryable_runtime_failures() {
        for err in [
            HardwareEncoderError::BufferPoolExhausted {
                needed: 2,
                available: 0,
            },
            HardwareEncoderError::Timeout { timeout_ms: 250 },
        ] {
            assert_eq!(err.class(), HardwareErrorClass::RetryableRuntime);
            assert!(err.is_recoverable());
        }
    }

    #[test]
    fn keeps_configuration_and_runtime_errors_distinct() {
        assert_eq!(
            HardwareEncoderError::InvalidPreset {
                preset: "ultra".into(),
            }
            .class(),
            HardwareErrorClass::Configuration
        );
        assert_eq!(
            HardwareEncoderError::EncodeFailed("driver reset".into()).class(),
            HardwareErrorClass::FatalRuntime
        );
    }

    #[test]
    fn display_messages_include_context_fields() {
        let err = HardwareEncoderError::InvalidDimensions {
            width: 1920,
            height: 1081,
            reason: "height must be even".into(),
        };
        let message = err.to_string();
        assert!(message.contains("1920x1081"));
        assert!(message.contains("height must be even"));
    }
}
