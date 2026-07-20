//! Minimal VA-API encode counters.

use std::time::Instant;
#[derive(Debug, Clone)]
pub struct HardwareEncoderStats {
    pub backend: &'static str,
    pub frames_encoded: u64,
    pub bytes_encoded: u64,
    pub avg_encode_time_ms: f32,
    pub target_bitrate_kbps: u32,
    pub keyframes_encoded: u64,
    pub frames_skipped: u64,
    total_encode_ms: f64,
}
impl HardwareEncoderStats {
    pub fn new(backend: &'static str, target_bitrate_kbps: u32) -> Self {
        Self {
            backend,
            frames_encoded: 0,
            bytes_encoded: 0,
            avg_encode_time_ms: 0.0,
            target_bitrate_kbps,
            keyframes_encoded: 0,
            frames_skipped: 0,
            total_encode_ms: 0.0,
        }
    }
    pub fn record_frame(&mut self, elapsed_ms: f32, bytes: usize, keyframe: bool) {
        self.frames_encoded += 1;
        self.bytes_encoded = self.bytes_encoded.saturating_add(bytes as u64);
        self.total_encode_ms += f64::from(elapsed_ms);
        self.avg_encode_time_ms = (self.total_encode_ms / self.frames_encoded as f64) as f32;
        if keyframe {
            self.keyframes_encoded += 1;
        }
    }
    pub fn record_skip(&mut self) {
        self.frames_skipped += 1;
    }
}
impl Default for HardwareEncoderStats {
    fn default() -> Self {
        Self::new("unknown", 5000)
    }
}
pub struct EncodeTimer {
    started: Instant,
}
impl EncodeTimer {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }
    pub fn elapsed_ms(&self) -> f32 {
        self.started.elapsed().as_secs_f32() * 1000.0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_frames() {
        let mut s = HardwareEncoderStats::new("vaapi", 5000);
        s.record_frame(2.0, 100, true);
        s.record_frame(4.0, 50, false);
        assert_eq!(s.frames_encoded, 2);
        assert_eq!(s.bytes_encoded, 150);
        assert_eq!(s.keyframes_encoded, 1);
        assert_eq!(s.avg_encode_time_ms, 3.0);
    }
}
