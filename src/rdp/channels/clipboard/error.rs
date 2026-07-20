//! Clipboard orchestration errors.

pub use crate::rdp::channels::clipboard::core::ClipboardError as CoreClipboardError;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ClipboardError>;

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error(transparent)]
    Core(#[from] CoreClipboardError),
    #[error("clipboard provider: {0}")]
    PortalError(String),
    #[error("invalid clipboard state: {0}")]
    InvalidState(String),
    #[error("RDP clipboard channel: {0}")]
    RdpConnectionError(String),
    #[error("clipboard IPC: {0}")]
    DBus(String),
    #[error("clipboard event sender closed")]
    ChannelSend,
    #[error("clipboard event receiver closed")]
    ChannelReceive,
    #[error("clipboard loop suppressed")]
    LoopDetected,
    #[error("clipboard file I/O: {0}")]
    FileIoError(String),
    #[error("clipboard component not initialized")]
    NotInitialized,
    #[error("clipboard failure: {0}")]
    Unknown(String),
}

impl ClipboardError {
    pub fn io(error: std::io::Error) -> Self {
        Self::Core(CoreClipboardError::Io(error))
    }
}
