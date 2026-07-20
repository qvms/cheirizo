//! Backend service modules for portal capture, clipboard, and input flows.
//!
//! This namespace exposes protocol-aware service backends and re-exports
//! the input backend types used by the runtime portal wiring.

pub mod capture;
pub mod clipboard;
pub mod input;

// Re-export input backend types
pub use input::{
    AvailableProtocols, EisBridgeBackend, EisConfig, EisSession, InputBackend, InputBackendConfig,
    InputProtocol, ProtocolDetector, WlrConfig, WlrInputBackend, create_input_backend,
};
