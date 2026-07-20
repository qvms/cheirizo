//! Performance control surface for runtime graphics loops.
//!
//! The display pipeline uses these helpers together:
//! - `AdaptiveFpsController` decides whether another frame should be captured
//!   based on recent damage/activity;
//! - `LatencyGovernor` decides whether accumulated damage should be encoded now
//!   or batched according to the selected latency mode.

mod adaptive_fps;
mod latency_governor;

pub use adaptive_fps::{AdaptiveFpsConfig, AdaptiveFpsController, DamageRatio};
pub use latency_governor::{EncodingDecision, LatencyGovernor, LatencyMode};
