//! Standard XDG ScreenCast portal capability snapshot.

use anyhow::{Context, Result};
use ashpd::desktop::screencast::{CursorMode as PortalCursor, SourceType as PortalSource};
use zbus::{Connection, names::InterfaceName};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMode {
    Hidden,
    Embedded,
    Metadata,
}
impl From<PortalCursor> for CursorMode {
    fn from(v: PortalCursor) -> Self {
        match v {
            PortalCursor::Hidden => Self::Hidden,
            PortalCursor::Embedded => Self::Embedded,
            PortalCursor::Metadata => Self::Metadata,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    Monitor,
    Window,
    Virtual,
}
impl From<PortalSource> for SourceType {
    fn from(v: PortalSource) -> Self {
        match v {
            PortalSource::Monitor => Self::Monitor,
            PortalSource::Window => Self::Window,
            PortalSource::Virtual => Self::Virtual,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PortalCapabilities {
    pub version: u32,
    pub supports_screencast: bool,
    pub available_cursor_modes: Vec<CursorMode>,
    pub available_source_types: Vec<SourceType>,
    pub backend: Option<String>,
}
impl PortalCapabilities {
    pub async fn probe() -> Result<Self> {
        let bus = Connection::session()
            .await
            .context("connect to session D-Bus")?;
        let version = property::<u32>(&bus, "version").await?;
        let source_flags = property::<u32>(&bus, "AvailableSourceTypes")
            .await
            .unwrap_or(0);
        let cursor_flags = property::<u32>(&bus, "AvailableCursorModes")
            .await
            .unwrap_or(0);
        Ok(Self {
            version,
            supports_screencast: true,
            available_cursor_modes: cursor_modes(cursor_flags),
            available_source_types: source_types(source_flags),
            backend: None,
        })
    }
    pub fn supports_metadata_cursor(&self) -> bool {
        self.available_cursor_modes.contains(&CursorMode::Metadata)
    }
    pub fn supports_monitor_capture(&self) -> bool {
        self.available_source_types.contains(&SourceType::Monitor)
    }
    pub fn supports_window_capture(&self) -> bool {
        self.available_source_types.contains(&SourceType::Window)
    }
}
async fn property<T>(bus: &Connection, name: &str) -> Result<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    let proxy = zbus::fdo::PropertiesProxy::builder(bus)
        .destination("org.freedesktop.portal.Desktop")?
        .path("/org/freedesktop/portal/desktop")?
        .build()
        .await?;
    let interface = InterfaceName::try_from("org.freedesktop.portal.ScreenCast")?;
    T::try_from(proxy.get(interface, name).await?).map_err(Into::into)
}
fn source_types(bits: u32) -> Vec<SourceType> {
    [
        (1, SourceType::Monitor),
        (2, SourceType::Window),
        (4, SourceType::Virtual),
    ]
    .into_iter()
    .filter_map(|(bit, value)| (bits & bit != 0).then_some(value))
    .collect()
}
fn cursor_modes(bits: u32) -> Vec<CursorMode> {
    [
        (1, CursorMode::Hidden),
        (2, CursorMode::Embedded),
        (4, CursorMode::Metadata),
    ]
    .into_iter()
    .filter_map(|(bit, value)| (bits & bit != 0).then_some(value))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flags_are_independent() {
        assert_eq!(
            source_types(3),
            vec![SourceType::Monitor, SourceType::Window]
        );
        assert_eq!(
            cursor_modes(6),
            vec![CursorMode::Embedded, CursorMode::Metadata]
        );
    }
    #[test]
    fn default_is_unavailable() {
        assert!(!PortalCapabilities::default().supports_screencast);
    }
}
