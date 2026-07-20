//! Shared clipboard primitives for wrdp CLIPRDR handling.
//!
//! This module centralizes format conversion, transfer policy, loop detection,
//! and sanitization used by the clipboard channel runtime path.

mod error;
mod sink;
mod transfer;

pub mod formats;
pub mod image;
pub mod loop_detector;
pub mod sanitize;

pub use error::{ClipboardError, ClipboardResult};
pub use formats::{
    ClipboardFormat, FileDescriptor, FileDescriptorFlags, FormatConverter,
    build_file_group_descriptor_w,
};
pub use loop_detector::{ClipboardSource, LoopDetectionConfig, LoopDetector};
pub use sink::{
    ClipboardChange, ClipboardChangeReceiver, ClipboardChangeReceiverInner, ClipboardSink, FileInfo,
};
pub use transfer::{
    DEFAULT_CHUNK_SIZE, DEFAULT_MAX_SIZE, DEFAULT_TIMEOUT_MS, TransferConfig, TransferEngine,
    TransferProgress, TransferState,
};

/// Prelude re-export for clipboard channel internals.
pub mod prelude {
    pub use super::formats::{mime_to_rdp_formats, rdp_format_to_mime};
    pub use super::{
        ClipboardChange, ClipboardError, ClipboardResult, ClipboardSink, FormatConverter,
        LoopDetector,
    };
}
