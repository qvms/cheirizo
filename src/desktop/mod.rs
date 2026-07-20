//! Local desktop integration boundary.
//!
//! The RDP server consumes frames, input injection, and compositor lifecycle
//! through this namespace instead of depending directly on Wayland/PipeWire/XDG
//! Desktop Portal APIs. Keeping these details here prevents endpoint channel
//! code from growing compositor-specific policy.

pub mod compositor;
pub mod pipewire;
pub mod portal;
