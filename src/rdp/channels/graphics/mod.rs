//! Graphics channel module boundary for wrdp runtime.
//!
//! Exposes bitmap conversion, damage tracking, EGFX transport, and
//! performance helpers used by the display/graphics pipeline.

pub(crate) mod bitmap;
pub mod damage;
pub mod egfx;
pub mod performance;
