//! RDP input-event translation surface for runtime session injection.
//!
//! Provides input-path primitives used by `wrdp`: monitor geometry,
//! scancode translation, coordinate mapping, and keyboard/mouse helpers
//! for compositor input routing.

mod coordinates;
mod error;
mod keyboard;
mod mapper;
mod mouse;

pub(crate) use coordinates::CoordinateTransformer;
pub use coordinates::MonitorInfo;
pub(crate) use error::InputError;
pub(crate) use keyboard::{KeyboardEvent, KeyboardHandler};
pub(crate) use mouse::{MouseButton, MouseEvent, MouseHandler};
