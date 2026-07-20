//! Top-level RDP runtime module boundaries.
//!
//! `rdp::server` handles listener/connection/channel orchestration,
//! `rdp::session` handles desktop-session lifecycle and supervision, and
//! `rdp::channels` contains endpoint-facing RDP channel implementations.

pub mod channels;
pub mod server;
pub mod session;
