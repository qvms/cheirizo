//! Damage-region detection for graphics update planning.
//!
//! This module provides tile-based frame differencing and region coalescing so
//! the graphics pipeline can focus encoding and transport work on changed areas.
mod compare;
mod config;
mod detector;
mod region;
mod regions;
mod stats;

pub use config::DamageConfig;
pub use detector::DamageDetector;
pub use region::DamageRegion;
pub use stats::DamageStats;

#[cfg(test)]
mod tests;
