//! `wrdp` library crate.
//!
//! The crate is organized around domain ownership rather than a flat
//! modules:
//! - `rdp::server` owns listener setup, connection binding, lifecycle, and
//!   RDP-server orchestration;
//! - `rdp::session` owns managed desktop session backends and supervision
//!   status;
//! - `rdp::channels` owns endpoint channel implementations for graphics, audio,
//!   input, and clipboard;
//! - `desktop` owns local compositor and XDG Desktop Portal integration;
//! - `services`, `security`, `runtime`, and `sesman` provide supporting runtime
//!   and integration concerns.
//!
//! Most modules are public because the `wrdp` binaries share this crate rather
//! than duplicating protocol and runtime glue. Low-level Portal and PipeWire
//! shims stay crate-private.

#![warn(clippy::all)]

pub mod auth;
pub mod config;
pub mod desktop;
pub mod rdp;
pub mod runtime;
pub mod security;
pub mod services;
pub mod sesman;

/// Minimal portal stream metadata used at the RDP/session boundary.
///
/// The full XDG Desktop Portal implementation lives under
/// `desktop::portal::xdg_desktop`; these lightweight types avoid depending on
/// separate portal helper layers just to pass monitor stream metadata
/// between the CLI/session and server paths.
pub mod portal {
    #[derive(Debug, Clone)]
    pub struct StreamInfo {
        /// PipeWire node identifier returned by XDG Desktop Portal ScreenCast.
        pub node_id: u32,
        /// Top-left stream position in the compositor's global monitor layout.
        pub position: (i32, i32),
        /// Stream size in physical pixels after portal/compositor negotiation.
        pub size: (u32, u32),
        /// Portal source class used to map stream metadata into RDP monitor data.
        pub source_type: SourceType,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SourceType {
        /// Full monitor or virtual-output capture.
        Monitor,
        /// Window-scoped capture, retained for portal API completeness.
        Window,
        /// Synthetic/virtual source exposed by the compositor or portal backend.
        Virtual,
    }
}

/// PipeWire types used internally.
pub(crate) mod pipewire {
    pub(crate) use crate::desktop::pipewire::{PipeWireThreadManager, VideoFrame};
}
