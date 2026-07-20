//! Error types for RDP input-channel handling.
//!
//! Defines failures surfaced by input translation, coordinate mapping,
//! layout/state handling, and channel event flow.

use thiserror::Error;

/// Result type for input operations.
pub(crate) type Result<T> = std::result::Result<T, InputError>;

/// Input-module error variants.
#[derive(Error, Debug)]
pub enum InputError {
    /// Portal remote desktop error
    #[error("Portal remote desktop error: {0}")]
    PortalError(String),

    /// Scancode translation error
    #[error("Scancode translation failed: {0}")]
    ScancodeTranslationFailed(String),

    /// Unknown scancode
    #[error("Unknown scancode: 0x{0:04X}")]
    UnknownScancode(u16),

    /// Unknown keycode
    #[error("Unknown keycode: {0}")]
    UnknownKeycode(u32),

    /// Coordinate transformation error
    #[error("Coordinate transformation error: {0}")]
    CoordinateTransformError(String),

    /// Monitor not found
    #[error("Monitor not found: {0}")]
    MonitorNotFound(u32),

    /// Invalid coordinate
    #[error("Invalid coordinate: ({0}, {1})")]
    InvalidCoordinate(f64, f64),

    /// Invalid monitor configuration
    #[error("Invalid monitor configuration: {0}")]
    InvalidMonitorConfig(String),

    /// Layout error
    #[error("Keyboard layout error: {0}")]
    LayoutError(String),

    /// Layout not found
    #[error("Layout not found: {0}")]
    LayoutNotFound(String),

    /// XKB error
    #[error("XKB error: {0}")]
    XkbError(String),

    /// Event queue full
    #[error("Event queue is full")]
    EventQueueFull,

    /// Event send error
    #[error("Failed to send event")]
    EventSendFailed,

    /// Event receive error
    #[error("Failed to receive event")]
    EventReceiveFailed,

    /// Input latency too high
    #[error("Input latency too high: {0}ms (max: {1}ms)")]
    LatencyTooHigh(u64, u64),

    /// Invalid state
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// Portal session error
    #[error("Portal session error: {0}")]
    PortalSessionError(String),

    /// DBus error
    #[error("DBus error: {0}")]
    DBusError(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid key event
    #[error("Invalid key event: {0}")]
    InvalidKeyEvent(String),

    /// Invalid mouse event
    #[error("Invalid mouse event: {0}")]
    InvalidMouseEvent(String),

    /// Unknown error
    #[error("Unknown error: {0}")]
    Unknown(String),
}
