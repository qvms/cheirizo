//! Typed portal backend settings resolved once during daemon startup.

use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result};

use crate::desktop::portal::xdg_desktop::{
    CaptureProtocol,
    services::{
        capture::CapturePreference,
        clipboard::{ClipboardPreference, ClipboardProtocol},
        input::{InputBackendConfig, InputProtocol},
    },
};

use super::Config;

const ENV_KEYS: [&str; 8] = [
    "XDP_GENERIC_CAPTURE_PROTOCOL",
    "XDP_GENERIC_CAPTURE_NO_FALLBACK",
    "XDP_GENERIC_CAPTURE_TIMEOUT_MS",
    "XDP_GENERIC_CLIPBOARD_PROTOCOL",
    "XDP_GENERIC_CLIPBOARD_NO_FALLBACK",
    "XDP_GENERIC_INPUT_PROTOCOL",
    "XDP_GENERIC_INPUT_NO_FALLBACK",
    "XDP_GENERIC_EIS_SOCKET",
];

#[derive(Debug, Clone)]
pub struct PortalStartupSettings {
    pub capture: CapturePreference,
    pub clipboard: ClipboardPreference,
    pub input: InputBackendConfig,
}

impl PortalStartupSettings {
    pub fn from_process(config: &Config) -> Result<Self> {
        let environment = ENV_KEYS
            .into_iter()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| (key.to_string(), value))
            })
            .collect();
        Self::resolve(config, &environment)
    }

    pub fn resolve(config: &Config, environment: &HashMap<String, String>) -> Result<Self> {
        let mut capture = CapturePreference {
            preferred: capture_protocol(&config.capture.protocol)?,
            allow_fallback: config.capture.allow_fallback,
            handshake_timeout_ms: config.capture.handshake_timeout_ms,
            direct_frame_channel: true,
            ..CapturePreference::default()
        };
        let mut clipboard = ClipboardPreference {
            preferred: clipboard_protocol(&config.clipboard.protocol)?,
            allow_fallback: config.clipboard.allow_fallback,
        };
        let mut input = InputBackendConfig {
            preferred: InputProtocol::WlrVirtualInput,
            ..InputBackendConfig::default()
        };

        if let Some(value) = environment.get("XDP_GENERIC_CAPTURE_PROTOCOL") {
            capture.preferred = capture_protocol(value)?;
        }
        if environment.contains_key("XDP_GENERIC_CAPTURE_NO_FALLBACK") {
            capture.allow_fallback = false;
        }
        if let Some(value) = environment.get("XDP_GENERIC_CAPTURE_TIMEOUT_MS") {
            capture.handshake_timeout_ms = value
                .parse()
                .context("XDP_GENERIC_CAPTURE_TIMEOUT_MS must be an integer")?;
        }
        if let Some(value) = environment.get("XDP_GENERIC_CLIPBOARD_PROTOCOL") {
            clipboard.preferred = clipboard_protocol(value)?;
        }
        if environment.contains_key("XDP_GENERIC_CLIPBOARD_NO_FALLBACK") {
            clipboard.allow_fallback = false;
        }
        if let Some(value) = environment.get("XDP_GENERIC_INPUT_PROTOCOL") {
            input.preferred = match value.as_str() {
                "eis" => InputProtocol::Eis,
                "wlr" => InputProtocol::WlrVirtualInput,
                other => anyhow::bail!("unsupported input protocol: {other}"),
            };
        }
        if environment.contains_key("XDP_GENERIC_INPUT_NO_FALLBACK") {
            input.allow_fallback = false;
        }
        if let Some(value) = environment.get("XDP_GENERIC_EIS_SOCKET") {
            input.eis.socket_path = Some(PathBuf::from(value));
        }

        Ok(Self {
            capture,
            clipboard,
            input,
        })
    }
}

fn capture_protocol(value: &str) -> Result<Option<CaptureProtocol>> {
    match value {
        "auto" => Ok(None),
        "ext" | "ext-image-copy-capture" => Ok(Some(CaptureProtocol::ExtImageCopyCapture)),
        "wlr" | "wlr-screencopy" => Ok(Some(CaptureProtocol::WlrScreencopy)),
        other => anyhow::bail!("unsupported capture protocol: {other}"),
    }
}

fn clipboard_protocol(value: &str) -> Result<Option<ClipboardProtocol>> {
    match value {
        "auto" => Ok(None),
        "ext" | "ext-data-control" => Ok(Some(ClipboardProtocol::ExtDataControl)),
        "wlr" | "wlr-data-control" => Ok(Some(ClipboardProtocol::WlrDataControl)),
        other => anyhow::bail!("unsupported clipboard protocol: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_values_become_typed_backend_settings() {
        let mut config = Config::default();
        config.capture.protocol = "ext".into();
        config.capture.allow_fallback = false;
        config.capture.handshake_timeout_ms = 2300;
        config.clipboard.protocol = "wlr".into();

        let settings = PortalStartupSettings::resolve(&config, &HashMap::new()).unwrap();
        assert_eq!(
            settings.capture.preferred,
            Some(CaptureProtocol::ExtImageCopyCapture)
        );
        assert!(!settings.capture.allow_fallback);
        assert_eq!(settings.capture.handshake_timeout_ms, 2300);
        assert_eq!(
            settings.clipboard.preferred,
            Some(ClipboardProtocol::WlrDataControl)
        );
        assert_eq!(settings.input.preferred, InputProtocol::WlrVirtualInput);
    }

    #[test]
    fn environment_snapshot_has_precedence_without_mutating_process_state() {
        let config = Config::default();
        let environment = HashMap::from([
            ("XDP_GENERIC_CAPTURE_PROTOCOL".into(), "wlr".into()),
            ("XDP_GENERIC_CAPTURE_NO_FALLBACK".into(), "0".into()),
            ("XDP_GENERIC_CAPTURE_TIMEOUT_MS".into(), "900".into()),
            ("XDP_GENERIC_CLIPBOARD_PROTOCOL".into(), "ext".into()),
            ("XDP_GENERIC_INPUT_PROTOCOL".into(), "eis".into()),
            ("XDP_GENERIC_EIS_SOCKET".into(), "/run/user/1000/eis".into()),
        ]);

        let settings = PortalStartupSettings::resolve(&config, &environment).unwrap();
        assert_eq!(
            settings.capture.preferred,
            Some(CaptureProtocol::WlrScreencopy)
        );
        assert!(!settings.capture.allow_fallback);
        assert_eq!(settings.capture.handshake_timeout_ms, 900);
        assert_eq!(
            settings.clipboard.preferred,
            Some(ClipboardProtocol::ExtDataControl)
        );
        assert_eq!(settings.input.preferred, InputProtocol::Eis);
        assert_eq!(
            settings.input.eis.socket_path,
            Some(PathBuf::from("/run/user/1000/eis"))
        );
    }

    #[test]
    fn invalid_override_is_rejected_at_startup() {
        let config = Config::default();
        let environment =
            HashMap::from([("XDP_GENERIC_CAPTURE_PROTOCOL".into(), "invented".into())]);
        assert!(PortalStartupSettings::resolve(&config, &environment).is_err());
    }
}
