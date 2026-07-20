//! Clipboard channel surface for wrdp runtime synchronization.
//!
//! Combines reusable format/transfer primitives with IronRDP CLIPRDR wiring and
//! compositor-provider policy. Runtime clipboard format/data/file events flow
//! through `ClipboardOrchestrator`, translate via `core`, and delegate local
//! clipboard ownership to the active provider.

// Server-specific modules (policy and orchestration)
pub mod core;
pub mod error;
pub mod ironrdp_backend;
pub mod manager;
pub mod provider;
pub mod providers;
pub mod sync;

// Shared clipboard primitives plus server-facing compatibility exports
pub use crate::rdp::channels::clipboard::core::formats::{
    mime_to_rdp_formats as lib_mime_to_rdp_formats, rdp_format_to_mime as lib_rdp_format_to_mime,
};
pub use crate::rdp::channels::clipboard::core::{
    // Error types (base)
    ClipboardError as CoreClipboardError,
    ClipboardFormat,
    FormatConverter,
    LoopDetectionConfig,
    LoopDetector,
    TransferConfig,
    TransferEngine,
    TransferProgress,
    TransferState,
};
pub use error::{ClipboardError, Result};
pub use ironrdp_backend::{
    ClipboardEvent as RdpClipboardEvent, ClipboardEventReceiver, ClipboardEventSender,
    ClipboardGeneralCapabilityFlags, RdpCliprdrBackend, RdpCliprdrFactory as LibRdpCliprdrFactory,
    WrdpCliprdrFactory,
};
pub use manager::{ClipboardEvent, ClipboardOrchestrator, ClipboardOrchestratorConfig};
pub use provider::{ClipboardProvider, ClipboardProviderEvent};
#[cfg(feature = "portal-generic")]
pub use providers::DataControlClipboardProvider;
pub use sync::{ClipboardState, SyncDirection, SyncManager};

/// Map a registered Windows clipboard name to its stable MIME representation.
pub(crate) fn format_name_to_mime(name: &str) -> Option<&'static str> {
    match name {
        "FileGroupDescriptorW" | "FileGroupDescriptor" => Some("text/uri-list"),
        "HTML Format" => Some("text/html"),
        "Rich Text Format" => Some("text/rtf"),
        _ => None,
    }
}

pub(crate) fn rdp_formats_to_mimes(formats: &[ClipboardFormat]) -> Vec<String> {
    let mut output = Vec::new();
    for format in formats {
        let mime = lib_rdp_format_to_mime(format.id)
            .or_else(|| format.name.as_deref().and_then(format_name_to_mime));
        if let Some(mime) = mime {
            if !output.iter().any(|known| known == mime) {
                output.push(mime.to_string());
            }
            if mime == "text/uri-list"
                && !output
                    .iter()
                    .any(|known| known == "x-special/gnome-copied-files")
            {
                output.push("x-special/gnome-copied-files".to_string());
            }
        }
    }
    if output.is_empty() && !formats.is_empty() {
        output.push("text/plain".to_string());
    }
    output
}
