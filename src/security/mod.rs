//! Security boundary for wrdp listener bootstrap and identity material.
//!
//! Runtime credential checks stay in `crate::auth`; this module intentionally
//! re-exports those validators together with TLS/certificate helpers so startup
//! wiring can import one security surface while session/auth internals remain
//! isolated in their own modules.

pub mod tls;

pub use crate::auth::{
    PamValidator, StaticPasswordValidator, hash_static_password, validate_username,
};
pub use tls::TlsConfig;
