//! Conservative compositor preferences and documented workarounds.
//!
//! Profiles never grant protocol capabilities. Observed Wayland globals and
//! standard portal properties remain authoritative.

use super::capabilities::{BufferType, CaptureBackend, CompositorType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quirk {
    RequiresWaylandSession,
    PoorDmaBufSupport,
    NeedsExplicitCursorComposite,
    InconsistentFrameTiming,
    InaccurateScreenSize,
    RestartCaptureOnResize,
    MultiMonitorPositionQuirk,
    LimitedBufferFormats,
    SessionTimeoutOnIdle,
    ColorSpaceQuirk,
    ForceAvc420,
    ExtCaptureIncomplete,
}
impl Quirk {
    pub const fn description(&self) -> &'static str {
        match self {
            Self::RequiresWaylandSession => "requires Wayland",
            Self::PoorDmaBufSupport => "avoid DMA-BUF",
            Self::NeedsExplicitCursorComposite => "composite cursor",
            Self::InconsistentFrameTiming => "irregular frame timing",
            Self::InaccurateScreenSize => "verify screen size",
            Self::RestartCaptureOnResize => "restart capture on resize",
            Self::MultiMonitorPositionQuirk => "verify monitor positions",
            Self::LimitedBufferFormats => "limited buffer formats",
            Self::SessionTimeoutOnIdle => "portal may expire while idle",
            Self::ColorSpaceQuirk => "verify color metadata",
            Self::ForceAvc420 => "disable AVC444",
            Self::ExtCaptureIncomplete => "avoid ext-image-copy-capture",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompositorProfile {
    pub compositor: CompositorType,
    pub wayland_protocols: Vec<String>,
    pub portal_backend: Option<String>,
    pub recommended_capture: CaptureBackend,
    pub recommended_buffer_type: BufferType,
    pub supports_damage_hints: bool,
    pub supports_explicit_sync: bool,
    pub quirks: Vec<Quirk>,
    pub recommended_fps_cap: u32,
}
impl Default for CompositorProfile {
    fn default() -> Self {
        Self {
            compositor: CompositorType::Unknown { session_info: None },
            wayland_protocols: vec![],
            portal_backend: None,
            recommended_capture: CaptureBackend::Portal,
            recommended_buffer_type: BufferType::MemFd,
            supports_damage_hints: false,
            supports_explicit_sync: false,
            quirks: vec![Quirk::PoorDmaBufSupport],
            recommended_fps_cap: 30,
        }
    }
}
impl CompositorProfile {
    pub fn for_compositor(compositor: &CompositorType) -> Self {
        let (capture, buffer, fps, quirks) = match compositor {
            CompositorType::Sway { .. } | CompositorType::Wlroots { .. } => (
                CaptureBackend::WlrScreencopy,
                BufferType::DmaBuf,
                60,
                vec![Quirk::NeedsExplicitCursorComposite],
            ),
            CompositorType::Hyprland { .. } => (
                CaptureBackend::WlrScreencopy,
                BufferType::DmaBuf,
                60,
                vec![
                    Quirk::NeedsExplicitCursorComposite,
                    Quirk::InconsistentFrameTiming,
                ],
            ),
            CompositorType::Kde { .. }
            | CompositorType::Cosmic
            | CompositorType::Smithay { .. } => {
                (CaptureBackend::Portal, BufferType::DmaBuf, 60, vec![])
            }
            CompositorType::Gnome { .. } => (
                CaptureBackend::Portal,
                BufferType::MemFd,
                30,
                vec![Quirk::RequiresWaylandSession, Quirk::RestartCaptureOnResize],
            ),
            CompositorType::Weston => (
                CaptureBackend::Portal,
                BufferType::MemFd,
                30,
                vec![Quirk::LimitedBufferFormats],
            ),
            CompositorType::Unknown { .. } => (
                CaptureBackend::Portal,
                BufferType::MemFd,
                30,
                vec![
                    Quirk::PoorDmaBufSupport,
                    Quirk::NeedsExplicitCursorComposite,
                ],
            ),
        };
        Self {
            compositor: compositor.clone(),
            wayland_protocols: vec![],
            portal_backend: None,
            recommended_capture: capture,
            recommended_buffer_type: buffer,
            supports_damage_hints: false,
            supports_explicit_sync: false,
            quirks,
            recommended_fps_cap: fps,
        }
    }
    pub fn has_quirk(&self, quirk: &Quirk) -> bool {
        self.quirks.contains(quirk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_do_not_claim_protocols() {
        let p = CompositorProfile::for_compositor(&CompositorType::Sway { version: None });
        assert!(p.wayland_protocols.is_empty());
        assert!(!p.supports_damage_hints);
        assert!(!p.supports_explicit_sync);
    }
    #[test]
    fn unknown_is_conservative() {
        let p = CompositorProfile::default();
        assert_eq!(p.recommended_buffer_type, BufferType::MemFd);
        assert!(p.has_quirk(&Quirk::PoorDmaBufSupport));
    }
}
