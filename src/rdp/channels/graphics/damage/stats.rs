/// Runtime counters for damage detection behavior.
#[derive(Debug, Clone, Default)]
pub struct DamageStats {
    pub frames_processed: u64,
    /// Frames with no detected damage.
    pub frames_skipped: u64,
    pub frames_full: u64,
    pub frames_partial: u64,
    pub total_damage_area: u64,
    pub total_frame_area: u64,
    /// Nanoseconds
    pub total_detection_time_ns: u64,
    /// Running ratio of damaged area to frame area.
    pub avg_damage_ratio: f32,
    pub avg_detection_time_ms: f32,
}

impl DamageStats {
    /// Estimated area reduction percentage from damage-based updates.
    pub fn bandwidth_reduction_percent(&self) -> f32 {
        if self.total_frame_area == 0 {
            return 0.0;
        }
        let ratio = self.total_damage_area as f32 / self.total_frame_area as f32;
        (1.0 - ratio) * 100.0
    }

    pub(super) fn record_frame(
        &mut self,
        frame_area: u64,
        damage_area: u64,
        detection_time_ns: u64,
    ) {
        self.frames_processed += 1;
        self.total_damage_area += damage_area;
        self.total_frame_area += frame_area;
        self.total_detection_time_ns += detection_time_ns;

        if damage_area == 0 {
            self.frames_skipped += 1;
        } else if damage_area >= frame_area * 9 / 10 {
            self.frames_full += 1;
        } else {
            self.frames_partial += 1;
        }

        self.update_averages();
    }

    fn update_averages(&mut self) {
        if self.frames_processed > 0 {
            self.avg_damage_ratio =
                self.total_damage_area as f32 / self.total_frame_area.max(1) as f32;
            self.avg_detection_time_ms = (self.total_detection_time_ns as f64
                / self.frames_processed as f64
                / 1_000_000.0) as f32;
        }
    }
}
