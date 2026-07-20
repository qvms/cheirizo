//! Damage-driven capture cadence policy.

use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveFpsConfig {
    pub enabled: bool,
    pub min_fps: u32,
    pub max_fps: u32,
    pub history_size: usize,
    pub high_activity_threshold: f32,
    pub medium_activity_threshold: f32,
    pub low_activity_threshold: f32,
    pub ramp_up_frames: usize,
    pub ramp_down_frames: usize,
}

impl Default for AdaptiveFpsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_fps: 5,
            max_fps: 30,
            history_size: 10,
            high_activity_threshold: 0.30,
            medium_activity_threshold: 0.10,
            low_activity_threshold: 0.01,
            ramp_up_frames: 2,
            ramp_down_frames: 5,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DamageRatio {
    pub ratio: f32,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActivityLevel {
    Static,
    Low,
    Medium,
    High,
}

impl ActivityLevel {
    pub const fn fps_multiplier(&self) -> f32 {
        match self {
            Self::Static => 0.0,
            Self::Low => 0.5,
            Self::Medium => 0.67,
            Self::High => 1.0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AdaptiveFpsStats {
    pub frames_processed: u64,
    pub frames_skipped: u64,
    pub time_at_static: Duration,
    pub time_at_low: Duration,
    pub time_at_medium: Duration,
    pub time_at_high: Duration,
    pub last_level_change: Option<Instant>,
}

pub struct AdaptiveFpsController {
    config: AdaptiveFpsConfig,
    current_fps: u32,
    activity_level: ActivityLevel,
    samples: VecDeque<f32>,
    candidate: ActivityLevel,
    candidate_frames: usize,
    last_capture: Instant,
    last_update: Instant,
    stats: AdaptiveFpsStats,
}

impl AdaptiveFpsController {
    pub fn new(mut config: AdaptiveFpsConfig) -> Self {
        config.min_fps = config.min_fps.max(1);
        config.max_fps = config.max_fps.max(config.min_fps);
        config.history_size = config.history_size.max(1);
        let now = Instant::now();
        Self {
            current_fps: config.max_fps,
            config,
            activity_level: ActivityLevel::High,
            samples: VecDeque::new(),
            candidate: ActivityLevel::High,
            candidate_frames: 0,
            last_capture: now,
            last_update: now,
            stats: AdaptiveFpsStats::default(),
        }
    }

    pub fn update(&mut self, damage_ratio: f32) {
        if !self.config.enabled {
            return;
        }
        let now = Instant::now();
        self.add_elapsed(now);
        self.samples.push_back(damage_ratio.clamp(0.0, 1.0));
        while self.samples.len() > self.config.history_size {
            self.samples.pop_front();
        }
        let mean = self.samples.iter().sum::<f32>() / self.samples.len() as f32;
        let target = self.classify(mean);
        if target == self.candidate {
            self.candidate_frames += 1;
        } else {
            self.candidate = target;
            self.candidate_frames = 1;
        }
        let needed = if target > self.activity_level {
            self.config.ramp_up_frames
        } else {
            self.config.ramp_down_frames
        }
        .max(1);
        if target != self.activity_level && self.candidate_frames >= needed {
            self.activity_level = target;
            self.stats.last_level_change = Some(now);
            self.candidate_frames = 0;
        }
        self.current_fps = self.target_fps();
        self.stats.frames_processed += 1;
    }

    pub fn should_capture_frame(&mut self) -> bool {
        let fps = if self.config.enabled {
            self.current_fps
        } else {
            self.config.max_fps
        }
        .max(1);
        if self.last_capture.elapsed() >= Duration::from_secs_f64(1.0 / f64::from(fps)) {
            self.last_capture = Instant::now();
            true
        } else {
            self.stats.frames_skipped += 1;
            false
        }
    }

    pub const fn current_fps(&self) -> u32 {
        self.current_fps
    }
    pub const fn activity_level(&self) -> ActivityLevel {
        self.activity_level
    }
    pub const fn stats(&self) -> &AdaptiveFpsStats {
        &self.stats
    }
    pub fn reset_stats(&mut self) {
        self.stats = AdaptiveFpsStats::default();
        self.last_update = Instant::now();
    }
    pub const fn is_enabled(&self) -> bool {
        self.config.enabled
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
        if !enabled {
            self.current_fps = self.config.max_fps;
        }
    }

    fn classify(&self, mean: f32) -> ActivityLevel {
        if mean >= self.config.high_activity_threshold {
            ActivityLevel::High
        } else if mean >= self.config.medium_activity_threshold {
            ActivityLevel::Medium
        } else if mean >= self.config.low_activity_threshold {
            ActivityLevel::Low
        } else {
            ActivityLevel::Static
        }
    }

    fn target_fps(&self) -> u32 {
        if self.activity_level == ActivityLevel::Static {
            return self.config.min_fps;
        }
        ((self.config.max_fps as f32 * self.activity_level.fps_multiplier()).round() as u32)
            .clamp(self.config.min_fps, self.config.max_fps)
    }

    fn add_elapsed(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_update);
        match self.activity_level {
            ActivityLevel::Static => self.stats.time_at_static += elapsed,
            ActivityLevel::Low => self.stats.time_at_low += elapsed,
            ActivityLevel::Medium => self.stats.time_at_medium += elapsed,
            ActivityLevel::High => self.stats.time_at_high += elapsed,
        }
        self.last_update = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_bounded() {
        let c = AdaptiveFpsConfig::default();
        assert!(c.enabled);
        assert!(c.min_fps <= c.max_fps);
    }
    #[test]
    fn sustained_activity_reaches_maximum() {
        let mut c = AdaptiveFpsController::new(AdaptiveFpsConfig::default());
        for _ in 0..12 {
            c.update(1.0);
        }
        assert_eq!(c.activity_level(), ActivityLevel::High);
        assert_eq!(c.current_fps(), 30);
    }
    #[test]
    fn sustained_idle_reaches_minimum() {
        let mut c = AdaptiveFpsController::new(AdaptiveFpsConfig::default());
        for _ in 0..20 {
            c.update(0.0);
        }
        assert_eq!(c.activity_level(), ActivityLevel::Static);
        assert_eq!(c.current_fps(), 5);
    }
    #[test]
    fn disabled_uses_fixed_maximum() {
        let mut cfg = AdaptiveFpsConfig::default();
        cfg.enabled = false;
        let c = AdaptiveFpsController::new(cfg);
        assert_eq!(c.current_fps(), 30);
    }
}
