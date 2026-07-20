//! Color pipeline helpers for EGFX encoding paths.
//!
//! Provides conversion, color-space policy, and AVC444 packing utilities used
//! by the runtime graphics encoder flow.

pub(crate) mod convert;
pub(crate) mod space;
pub(crate) mod yuv444_packing;
