//! Encode scheduling policy for interactive, balanced and quality modes.

use serde::{Deserialize, Serialize};
use std::{
    str::FromStr,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LatencyMode {
    Interactive,
    #[default]
    Balanced,
    Quality,
}

impl LatencyMode {
    pub const fn target_latency_ms(&self) -> u32 {
        match self {
            Self::Interactive => 50,
            Self::Balanced => 100,
            Self::Quality => 300,
        }
    }
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Balanced => "balanced",
            Self::Quality => "quality",
        }
    }
    const fn policy(self) -> Policy {
        match self {
            Self::Interactive => Policy {
                threshold: 0.0,
                delay: Duration::from_millis(16),
                timeout: Duration::from_millis(10),
                adaptive: false,
            },
            Self::Balanced => Policy {
                threshold: 0.02,
                delay: Duration::from_millis(33),
                timeout: Duration::from_millis(20),
                adaptive: true,
            },
            Self::Quality => Policy {
                threshold: 0.05,
                delay: Duration::from_millis(100),
                timeout: Duration::from_millis(50),
                adaptive: true,
            },
        }
    }
}
impl std::fmt::Display for LatencyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.description())
    }
}
impl FromStr for LatencyMode {
    type Err = String;
    fn from_str(v: &str) -> Result<Self, Self::Err> {
        match v.to_ascii_lowercase().as_str() {
            "interactive" | "low" | "fast" => Ok(Self::Interactive),
            "balanced" | "default" | "normal" => Ok(Self::Balanced),
            "quality" | "high" | "slow" => Ok(Self::Quality),
            _ => Err(format!("unknown latency mode: {v}")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Policy {
    threshold: f32,
    delay: Duration,
    timeout: Duration,
    adaptive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingDecision {
    EncodeNow,
    EncodeKeepalive,
    EncodeBatch,
    EncodeTimeout,
    Skip,
    WaitForMore,
}
impl EncodingDecision {
    pub const fn should_encode(&self) -> bool {
        matches!(
            self,
            Self::EncodeNow | Self::EncodeKeepalive | Self::EncodeBatch | Self::EncodeTimeout
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct LatencyMetrics {
    pub capture_to_encode_avg_ms: f32,
    pub encode_duration_avg_ms: f32,
    pub total_latency_avg_ms: f32,
    pub frames_encoded: u64,
    pub frames_skipped: u64,
    pub batches_encoded: u64,
}

pub struct LatencyGovernor {
    mode: LatencyMode,
    policy: Policy,
    pending: f32,
    pending_since: Option<Instant>,
    pending_frames: u32,
    last_encode: Instant,
    metrics: LatencyMetrics,
}

impl LatencyGovernor {
    pub fn new(mode: LatencyMode) -> Self {
        Self {
            mode,
            policy: mode.policy(),
            pending: 0.0,
            pending_since: None,
            pending_frames: 0,
            last_encode: Instant::now(),
            metrics: LatencyMetrics::default(),
        }
    }
    pub fn should_encode_frame(&mut self, damage: f32) -> EncodingDecision {
        let damage = damage.clamp(0.0, 1.0);
        if damage > 0.0 {
            self.pending_since.get_or_insert_with(Instant::now);
            self.pending = (self.pending + damage).min(1.0);
            self.pending_frames += 1;
        }
        let elapsed = self.pending_since.map_or(Duration::ZERO, |t| t.elapsed());
        let choice = match self.mode {
            LatencyMode::Interactive if damage > 0.0 => EncodingDecision::EncodeNow,
            LatencyMode::Interactive if self.last_encode.elapsed() >= self.policy.delay => {
                EncodingDecision::EncodeKeepalive
            }
            LatencyMode::Interactive => EncodingDecision::Skip,
            LatencyMode::Balanced if reached(self.pending, self.policy.threshold) => {
                EncodingDecision::EncodeNow
            }
            LatencyMode::Balanced if self.pending > 0.0 && elapsed >= self.policy.delay => {
                EncodingDecision::EncodeTimeout
            }
            LatencyMode::Balanced => EncodingDecision::Skip,
            LatencyMode::Quality if reached(self.pending, self.policy.threshold) => {
                EncodingDecision::EncodeBatch
            }
            LatencyMode::Quality if self.pending > 0.0 && elapsed >= self.policy.delay => {
                EncodingDecision::EncodeTimeout
            }
            LatencyMode::Quality if self.pending > 0.0 => EncodingDecision::WaitForMore,
            LatencyMode::Quality => EncodingDecision::Skip,
        };
        if choice.should_encode() {
            self.metrics.frames_encoded += 1;
            if self.pending_frames > 1 {
                self.metrics.batches_encoded += 1;
            }
            self.pending = 0.0;
            self.pending_since = None;
            self.pending_frames = 0;
            self.last_encode = Instant::now();
        } else {
            self.metrics.frames_skipped += 1;
        }
        choice
    }
    pub fn record_encode_timing(&mut self, capture: f32, encode: f32) {
        const W: f32 = 0.1;
        self.metrics.capture_to_encode_avg_ms =
            ema(self.metrics.capture_to_encode_avg_ms, capture, W);
        self.metrics.encode_duration_avg_ms = ema(self.metrics.encode_duration_avg_ms, encode, W);
        self.metrics.total_latency_avg_ms =
            ema(self.metrics.total_latency_avg_ms, capture + encode, W);
    }
    pub const fn mode(&self) -> LatencyMode {
        self.mode
    }
    pub fn set_mode(&mut self, mode: LatencyMode) {
        self.mode = mode;
        self.policy = mode.policy();
        self.pending = 0.0;
        self.pending_since = None;
        self.pending_frames = 0;
    }
    pub const fn should_use_adaptive_fps(&self) -> bool {
        self.policy.adaptive
    }
    pub const fn encode_timeout(&self) -> Duration {
        self.policy.timeout
    }
    pub const fn metrics(&self) -> &LatencyMetrics {
        &self.metrics
    }
    pub fn reset_metrics(&mut self) {
        self.metrics = LatencyMetrics::default();
    }
    pub fn time_since_last_encode(&self) -> Duration {
        self.last_encode.elapsed()
    }
}
fn reached(value: f32, threshold: f32) -> bool {
    value + f32::EPSILON >= threshold
}

fn ema(old: f32, new: f32, weight: f32) -> f32 {
    if old == 0.0 {
        new
    } else {
        old * (1.0 - weight) + new * weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_parse() {
        assert_eq!("fast".parse(), Ok(LatencyMode::Interactive));
        assert!("bogus".parse::<LatencyMode>().is_err());
    }
    #[test]
    fn interactive_sends_damage() {
        let mut g = LatencyGovernor::new(LatencyMode::Interactive);
        assert_eq!(g.should_encode_frame(0.001), EncodingDecision::EncodeNow);
    }
    #[test]
    fn balanced_accumulates() {
        let mut g = LatencyGovernor::new(LatencyMode::Balanced);
        assert_eq!(g.should_encode_frame(0.01), EncodingDecision::Skip);
        assert_eq!(g.should_encode_frame(0.01), EncodingDecision::EncodeNow);
    }
    #[test]
    fn quality_batches() {
        let mut g = LatencyGovernor::new(LatencyMode::Quality);
        assert_eq!(g.should_encode_frame(0.02), EncodingDecision::WaitForMore);
        assert_eq!(g.should_encode_frame(0.03), EncodingDecision::EncodeBatch);
        assert_eq!(g.metrics().batches_encoded, 1);
    }
    #[test]
    fn encoding_variants_report_true() {
        assert!(EncodingDecision::EncodeTimeout.should_encode());
        assert!(!EncodingDecision::Skip.should_encode());
    }
}
