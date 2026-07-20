//! Clipboard provider backend modules.
//!
//! Provider modules adapt concrete local clipboard backends to the common
//! `ClipboardProvider` contract consumed by `ClipboardOrchestrator`.

#[cfg(feature = "portal-generic")]
pub mod data_control;

#[cfg(feature = "portal-generic")]
pub use data_control::DataControlClipboardProvider;
