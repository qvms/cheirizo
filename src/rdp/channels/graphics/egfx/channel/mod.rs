//! EGFX dynamic-channel wiring for the runtime server path.
//!
//! The server layer owns listener/session lifecycle. This module owns the
//! graphics-pipeline channel adapter: capability negotiation state, EGFX frame
//! dispatch, and server-output draining.

mod factory;
mod sender;

pub use factory::{
    EgfxChannelFactory, EgfxCodecPolicy, HandlerState, NegotiatedEgfxMode, SharedHandlerState,
};
pub use sender::EgfxChannelSender;

pub(crate) use sender::resize_with_primary_monitor;
