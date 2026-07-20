//! Managed session surface for wrdp runtime backends.
//!
//! The primary runtime path is the bundled `wrdp-compositor` supervised by
//! `wrdp-sesman`. The in-process `portal-generic` backend provides capture,
//! input, and clipboard integration through native Wayland protocols, while
//! deployment detection remains available for diagnostics/reporting.

pub mod backend;
pub mod deployment;
pub mod supervision;

pub mod backends {
    #[cfg(feature = "portal-generic")]
    pub mod portal_generic;
    #[cfg(feature = "portal-generic")]
    pub use portal_generic::PortalSessionBackend;
}

pub use backend::{DirectFrameReceiver, SessionHandle};
pub use deployment::{DeploymentContext, detect_deployment_context};
