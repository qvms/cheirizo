//! Managed compositor runtime constants.

/// Name of the managed compositor component in sesman state.
pub const MANAGED_COMPOSITOR_COMPONENT: &str = "wrdp-compositor";
/// Managed compositor launcher path.
pub const MANAGED_COMPOSITOR_COMMAND: &str = "/usr/lib/wrdp/wrdp-compositor";
/// Configuration directory containing rc.xml, menu.xml, and autostart.
pub const MANAGED_WRDP_COMPOSITOR_CONFIG_DIR: &str = "/etc/wrdp/labwc";
/// Wayland display name used by the managed compositor stack.
pub const MANAGED_WAYLAND_DISPLAY: &str = "wayland-0";
/// Full socket name exposed by the managed compositor.
pub const MANAGED_WAYLAND_SOCKET: &str = "wayland-0";
/// Lock file paired with the managed Wayland socket.
pub const MANAGED_WAYLAND_LOCK: &str = "wayland-0.lock";
