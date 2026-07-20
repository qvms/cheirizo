//! Observed desktop protocol state plus diagnostic compositor identity.

use super::{portal_caps::PortalCapabilities, profiles::CompositorProfile};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum CompositorType {
    Gnome { version: Option<String> },
    Kde { version: Option<String> },
    Sway { version: Option<String> },
    Hyprland { version: Option<String> },
    Weston,
    Cosmic,
    Smithay { name: String },
    Wlroots { name: String },
    Unknown { session_info: Option<String> },
}
impl CompositorType {
    pub fn name(&self) -> &str {
        match self {
            Self::Gnome { .. } => "GNOME",
            Self::Kde { .. } => "KDE Plasma",
            Self::Sway { .. } => "Sway",
            Self::Hyprland { .. } => "Hyprland",
            Self::Weston => "Weston",
            Self::Cosmic => "Cosmic",
            Self::Smithay { name } | Self::Wlroots { name } => name,
            Self::Unknown { .. } => "Unknown",
        }
    }
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Gnome { version }
            | Self::Kde { version }
            | Self::Sway { version }
            | Self::Hyprland { version } => version.as_deref(),
            _ => None,
        }
    }
    pub fn is_wlroots_based(&self) -> bool {
        matches!(
            self,
            Self::Sway { .. } | Self::Hyprland { .. } | Self::Wlroots { .. }
        )
    }
}
impl std::fmt::Display for CompositorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(v) = self.version() {
            write!(f, "{} {v}", self.name())
        } else {
            f.write_str(self.name())
        }
    }
}
#[derive(Debug, Clone)]
pub struct WaylandGlobal {
    pub interface: String,
    pub version: u32,
    pub name: u32,
}
impl WaylandGlobal {
    pub fn new(interface: impl Into<String>, version: u32, name: u32) -> Self {
        Self {
            interface: interface.into(),
            version,
            name,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BufferType {
    MemFd,
    DmaBuf,
    #[default]
    Any,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureBackend {
    #[default]
    Portal,
    WlrScreencopy,
    ExtImageCopyCapture,
}

#[derive(Debug, Clone)]
pub struct CompositorCapabilities {
    pub compositor: CompositorType,
    pub portal: PortalCapabilities,
    pub wayland_globals: Vec<WaylandGlobal>,
    pub profile: CompositorProfile,
    pub deployment: crate::rdp::session::DeploymentContext,
    versions: HashMap<String, u32>,
}
impl CompositorCapabilities {
    pub fn new(
        compositor: CompositorType,
        portal: PortalCapabilities,
        wayland_globals: Vec<WaylandGlobal>,
    ) -> Self {
        let versions = wayland_globals
            .iter()
            .map(|g| (g.interface.clone(), g.version))
            .collect::<HashMap<_, _>>();
        let has = |n: &str| versions.contains_key(n);
        let mut profile = CompositorProfile::for_compositor(&compositor);
        profile.supports_damage_hints = has("zwlr_screencopy_manager_v1");
        profile.supports_explicit_sync =
            has("wp_linux_drm_syncobj_manager_v1") || has("zwp_linux_explicit_synchronization_v1");
        profile.recommended_capture = if has("zwlr_screencopy_manager_v1") {
            CaptureBackend::WlrScreencopy
        } else if has("ext_image_copy_capture_manager_v1") {
            CaptureBackend::ExtImageCopyCapture
        } else {
            CaptureBackend::Portal
        };
        if !has("zwp_linux_dmabuf_v1") {
            profile.recommended_buffer_type = BufferType::MemFd;
        }
        Self {
            compositor,
            portal,
            wayland_globals,
            profile,
            deployment: crate::rdp::session::detect_deployment_context(),
            versions,
        }
    }
    pub fn has_protocol(&self, name: &str, min: u32) -> bool {
        self.versions.get(name).is_some_and(|v| *v >= min)
    }
    pub fn get_protocol_version(&self, name: &str) -> Option<u32> {
        self.versions.get(name).copied()
    }
    pub fn has_wlr_screencopy(&self) -> bool {
        self.has_protocol("zwlr_screencopy_manager_v1", 1)
    }
    pub fn has_ext_image_copy_capture(&self) -> bool {
        self.has_protocol("ext_image_copy_capture_manager_v1", 1)
    }
    pub fn has_fractional_scale(&self) -> bool {
        self.has_protocol("wp_fractional_scale_manager_v1", 1)
    }
    pub fn has_ext_data_control(&self) -> bool {
        self.has_protocol("ext_data_control_manager_v1", 1)
    }
    pub fn has_wlr_data_control(&self) -> bool {
        self.has_protocol("zwlr_data_control_manager_v1", 1)
    }
    pub fn has_any_data_control(&self) -> bool {
        self.has_ext_data_control() || self.has_wlr_data_control()
    }
    pub fn has_virtual_keyboard(&self) -> bool {
        self.has_protocol("zwp_virtual_keyboard_manager_v1", 1)
    }
    pub fn has_virtual_pointer(&self) -> bool {
        self.has_protocol("zwlr_virtual_pointer_manager_v1", 1)
    }
    pub fn has_virtual_input(&self) -> bool {
        self.has_virtual_keyboard() && self.has_virtual_pointer()
    }
    pub fn has_color_management(&self) -> bool {
        self.has_protocol("wp_color_management_v1", 1)
    }
    pub fn log_summary(&self) {
        tracing::info!(compositor=%self.compositor,portal_version=self.portal.version,globals=self.wayland_globals.len(),capture=?self.profile.recommended_capture,buffer=?self.profile.recommended_buffer_type,"desktop capabilities");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_versions_are_authoritative() {
        let c = CompositorCapabilities::new(
            CompositorType::Unknown { session_info: None },
            PortalCapabilities::default(),
            vec![WaylandGlobal::new("example", 2, 7)],
        );
        assert!(c.has_protocol("example", 2));
        assert!(!c.has_protocol("example", 3));
    }
}
