//! Small runtime decisions derived from observed desktop capabilities.

use crate::desktop::compositor::CompositorCapabilities;

#[derive(Debug, Clone)]
pub struct RuntimeCapabilities {
    pub damage_hints: bool,
    pub explicit_sync: bool,
    pub dmabuf: bool,
    pub native_input: bool,
    pub data_control: bool,
}
impl RuntimeCapabilities {
    pub fn from_compositor(c: &CompositorCapabilities) -> Self {
        Self {
            damage_hints: c.has_wlr_screencopy(),
            explicit_sync: c.has_protocol("wp_linux_drm_syncobj_manager_v1", 1)
                || c.has_protocol("zwp_linux_explicit_synchronization_v1", 1),
            dmabuf: c.has_protocol("zwp_linux_dmabuf_v1", 1),
            native_input: c.has_virtual_input(),
            data_control: c.has_any_data_control(),
        }
    }
    pub const fn should_enable_adaptive_fps(&self) -> bool {
        self.damage_hints
    }
}
