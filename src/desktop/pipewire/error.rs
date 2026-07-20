//! Errors returned by the PipeWire capture boundary.

use thiserror::Error;
pub type Result<T> = std::result::Result<T, PipeWireError>;
#[derive(Debug, Error)]
pub enum PipeWireError {
    #[error("PipeWire initialization: {0}")]
    InitializationFailed(String),
    #[error("PipeWire connection: {0}")]
    ConnectionFailed(String),
    #[error("PipeWire stream creation: {0}")]
    StreamCreationFailed(String),
    #[error("PipeWire format negotiation: {0}")]
    FormatNegotiationFailed(String),
    #[error("PipeWire buffer allocation: {0}")]
    BufferAllocationFailed(String),
    #[error("DMA-BUF import: {0}")]
    DmaBufImportFailed(String),
    #[error("frame extraction: {0}")]
    FrameExtractionFailed(String),
    #[error("stream {0} not found")]
    StreamNotFound(u32),
    #[error("stream limit exceeded ({0})")]
    TooManyStreams(usize),
    #[error("stream {0} stalled")]
    StreamStalled(u32),
    #[error("pixel format conversion: {0}")]
    FormatConversionFailed(String),
    #[error("PipeWire operation timed out")]
    Timeout,
    #[error("PipeWire permission denied")]
    PermissionDenied,
    #[error("no PipeWire buffers available")]
    NoBuffersAvailable,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("portal: {0}")]
    Portal(String),
    #[error("PipeWire command channel: {0}")]
    ThreadCommunicationFailed(String),
    #[error("PipeWire thread: {0}")]
    ThreadPanic(String),
}
