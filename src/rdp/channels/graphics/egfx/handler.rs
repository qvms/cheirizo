//! IronRDP EGFX negotiation callbacks.

use crate::rdp::channels::graphics::egfx::channel::{
    HandlerState, NegotiatedEgfxMode, SharedHandlerState,
};
use ironrdp_egfx::{
    pdu::{
        CapabilitiesAdvertisePdu, CapabilitiesV10Flags, CapabilitiesV81Flags,
        CapabilitiesV103Flags, CapabilitiesV104Flags, CapabilitiesV107Flags, CapabilitySet,
    },
    server::GraphicsPipelineHandler,
};
use std::sync::Arc;

pub struct WrdpGraphicsHandler {
    state: SharedHandlerState,
    avc420_only: bool,
    max_frames: u32,
}
impl WrdpGraphicsHandler {
    pub fn with_quirks(_width: u16, _height: u16, avc420_only: bool) -> Self {
        Self {
            state: Arc::new(tokio::sync::RwLock::new(None)),
            avc420_only,
            max_frames: 3,
        }
    }
    pub fn with_config(
        _width: u16,
        _height: u16,
        state: SharedHandlerState,
        avc420_only: bool,
        max_frames: u32,
    ) -> Self {
        Self {
            state,
            avc420_only,
            max_frames,
        }
    }
    fn publish(&self, value: Option<HandlerState>) {
        for _ in 0..50 {
            if let Ok(mut state) = self.state.try_write() {
                *state = value;
                return;
            }
            std::thread::yield_now();
        }
        tracing::warn!("EGFX negotiated state update was contended");
    }
}
impl GraphicsPipelineHandler for WrdpGraphicsHandler {
    fn capabilities_advertise(&mut self, pdu: &CapabilitiesAdvertisePdu) {
        tracing::debug!(sets = pdu.0.len(), "EGFX capabilities advertised");
    }
    fn on_ready(&mut self, cap: &CapabilitySet) {
        let (avc420, mut avc444, android) = codec_mode(cap);
        if self.avc420_only {
            avc444 = false
        }
        let mode = if avc444 {
            NegotiatedEgfxMode::Avc444
        } else if avc420 {
            NegotiatedEgfxMode::Avc420
        } else {
            NegotiatedEgfxMode::Planar
        };
        self.publish(Some(HandlerState {
            is_ready: true,
            negotiated_mode: Some(mode),
            is_avc420_enabled: avc420,
            is_avc444_enabled: avc444,
            needs_android_pointer_updates: android,
            primary_surface_id: None,
            dvc_channel_id: 0,
        }));
    }
    fn on_capability_negotiation_failed(&mut self) {
        self.publish(None)
    }
    fn on_close(&mut self) {
        self.publish(None)
    }
    fn max_frames_in_flight(&self) -> u32 {
        self.max_frames
    }
    fn preferred_capabilities(&self) -> Vec<CapabilitySet> {
        vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE,
            },
            CapabilitySet::V10 {
                flags: CapabilitiesV10Flags::SMALL_CACHE,
            },
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
            },
        ]
    }
}
fn codec_mode(cap: &CapabilitySet) -> (bool, bool, bool) {
    match cap {
        CapabilitySet::V8 { .. } => (false, false, true),
        CapabilitySet::V8_1 { flags } => {
            let avc = flags.contains(CapabilitiesV81Flags::AVC420_ENABLED);
            (avc, false, !avc)
        }
        CapabilitySet::V10 { flags } | CapabilitySet::V10_2 { flags } => {
            enabled(!flags.contains(CapabilitiesV10Flags::AVC_DISABLED))
        }
        CapabilitySet::V10_1 => (true, true, false),
        CapabilitySet::V10_3 { flags } => {
            enabled(!flags.contains(CapabilitiesV103Flags::AVC_DISABLED))
        }
        CapabilitySet::V10_4 { flags }
        | CapabilitySet::V10_5 { flags }
        | CapabilitySet::V10_6 { flags }
        | CapabilitySet::V10_6Err { flags } => {
            enabled(!flags.contains(CapabilitiesV104Flags::AVC_DISABLED))
        }
        CapabilitySet::V10_7 { flags } => {
            enabled(!flags.contains(CapabilitiesV107Flags::AVC_DISABLED))
        }
    }
}
const fn enabled(value: bool) -> (bool, bool, bool) {
    (value, value, !value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v81_requires_flag() {
        assert_eq!(
            codec_mode(&CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::empty()
            }),
            (false, false, true)
        );
    }
}
