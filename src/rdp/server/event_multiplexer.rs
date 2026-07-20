//! Shared graphics queue payload.
//!
//! Used by the server display path when EGFX is disabled so pre-converted
//! bitmap updates can be queued to the graphics drain task without repeating
//! conversion work.

/// Graphics frame update payload.
pub struct GraphicsFrame {
    pub iron_bitmap: ironrdp_server::BitmapUpdate,
    pub sequence: u64,
}

impl GraphicsFrame {
    pub fn new(iron_bitmap: ironrdp_server::BitmapUpdate, sequence: u64) -> Self {
        Self {
            iron_bitmap,
            sequence,
        }
    }
}
