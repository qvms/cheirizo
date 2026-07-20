use std::time::Instant;

use super::{
    DamageConfig, DamageRegion, DamageStats,
    compare::count_different_pixels,
    regions::{merge_regions, tiles_to_regions},
};

/// Damage detection engine for graphics update planning.
///
/// Compares consecutive frames and emits changed regions so downstream
/// encoding/transport can prioritize modified areas.
pub struct DamageDetector {
    config: DamageConfig,
    previous_frame: Option<Vec<u8>>,
    previous_dimensions: Option<(u32, u32)>,
    /// Reused between frames to avoid allocation
    tile_dirty: Vec<bool>,
    tiles_x: usize,
    tiles_y: usize,
    stats: DamageStats,
    /// Forces full-frame damage on the next `detect()` call.
    invalidated: bool,
}

impl DamageDetector {
    pub fn new(config: DamageConfig) -> Self {
        Self {
            config,
            previous_frame: None,
            previous_dimensions: None,
            tile_dirty: Vec::new(),
            tiles_x: 0,
            tiles_y: 0,
            stats: DamageStats::default(),
            invalidated: true,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(DamageConfig::default())
    }

    /// Returns empty when the frame matches the preceding frame and full-frame
    /// damage on the first call or after invalidation.
    ///
    /// `frame` must be BGRA pixel data (4 bytes per pixel).
    #[expect(
        clippy::unwrap_used,
        reason = "previous_frame is guaranteed Some after first frame check"
    )]
    pub fn detect(&mut self, frame: &[u8], width: u32, height: u32) -> Vec<DamageRegion> {
        let start = Instant::now();
        let frame_area = width as u64 * height as u64;
        let expected_len = (width as usize) * (height as usize) * 4;

        assert_eq!(
            frame.len(),
            expected_len,
            "Frame size mismatch: got {} bytes, expected {} for {}×{}",
            frame.len(),
            expected_len,
            width,
            height
        );

        let dimensions_changed = self
            .previous_dimensions
            .is_none_or(|(w, h)| w != width || h != height);

        if self.previous_frame.is_none() || self.invalidated || dimensions_changed {
            self.update_tile_grid(width, height);
            self.previous_frame = Some(frame.to_vec());
            self.previous_dimensions = Some((width, height));
            self.invalidated = false;

            self.stats
                .record_frame(frame_area, frame_area, start.elapsed().as_nanos() as u64);

            return vec![DamageRegion::full_frame(width, height)];
        }

        // Take ownership of previous frame temporarily to avoid borrow issues
        let mut prev_frame = self.previous_frame.take().unwrap();
        let regions = self.detect_changes(&prev_frame, frame, width, height);

        let damage_area: u64 = regions.iter().map(DamageRegion::area).sum();

        self.stats
            .record_frame(frame_area, damage_area, start.elapsed().as_nanos() as u64);

        // Store current frame for next comparison (reuse allocation)
        prev_frame.clear();
        prev_frame.extend_from_slice(frame);
        self.previous_frame = Some(prev_frame);

        regions
    }

    /// Call after resolution changes, keyframe boundaries, or other events
    /// that require a full refresh.
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }

    pub fn stats(&self) -> &DamageStats {
        &self.stats
    }

    pub fn reset_stats(&mut self) {
        self.stats = DamageStats::default();
    }

    pub fn config(&self) -> &DamageConfig {
        &self.config
    }

    /// Invalidates the detector, so the next frame is treated as full damage.
    pub fn set_config(&mut self, config: DamageConfig) {
        self.config = config;
        self.invalidate();
    }

    fn update_tile_grid(&mut self, width: u32, height: u32) {
        self.tiles_x = (width as usize).div_ceil(self.config.tile_size);
        self.tiles_y = (height as usize).div_ceil(self.config.tile_size);
        let total_tiles = self.tiles_x * self.tiles_y;

        if self.tile_dirty.len() != total_tiles {
            self.tile_dirty = vec![false; total_tiles];
        }
    }

    fn detect_changes(
        &mut self,
        prev: &[u8],
        curr: &[u8],
        width: u32,
        height: u32,
    ) -> Vec<DamageRegion> {
        let tile_size = self.config.tile_size;
        let stride = (width as usize) * 4;
        let pixel_threshold = self.config.pixel_threshold;
        for flag in &mut self.tile_dirty {
            *flag = false;
        }

        for ty in 0..self.tiles_y {
            for tx in 0..self.tiles_x {
                let tile_x = tx * tile_size;
                let tile_y = ty * tile_size;

                // Calculate actual tile dimensions (may be smaller at edges)
                let tile_width = tile_size.min((width as usize).saturating_sub(tile_x));
                let tile_height = tile_size.min((height as usize).saturating_sub(tile_y));

                if tile_width == 0 || tile_height == 0 {
                    continue;
                }

                let diff_count = self.compare_tile(
                    prev,
                    curr,
                    tile_x,
                    tile_y,
                    tile_width,
                    tile_height,
                    stride,
                    pixel_threshold,
                );

                let tile_pixels = (tile_width * tile_height) as u32;
                let diff_threshold_count = (tile_pixels as f32 * self.config.diff_threshold) as u32;
                let idx = ty * self.tiles_x + tx;
                self.tile_dirty[idx] = diff_count > diff_threshold_count;
            }
        }

        let mut regions = tiles_to_regions(
            &self.tile_dirty,
            self.tiles_x,
            self.tiles_y,
            tile_size,
            width,
            height,
        );

        regions = merge_regions(regions, self.config.merge_distance);
        regions.retain(|r| r.area() >= self.config.min_region_area);

        regions
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "tile comparison needs geometry + data refs"
    )]
    fn compare_tile(
        &self,
        prev: &[u8],
        curr: &[u8],
        tile_x: usize,
        tile_y: usize,
        tile_width: usize,
        tile_height: usize,
        stride: usize,
        pixel_threshold: u8,
    ) -> u32 {
        let mut total_diff = 0u32;
        let bytes_per_row = tile_width * 4;

        for row in 0..tile_height {
            let y = tile_y + row;
            let offset = y * stride + tile_x * 4;

            if offset + bytes_per_row > prev.len() || offset + bytes_per_row > curr.len() {
                continue;
            }

            let prev_row = &prev[offset..offset + bytes_per_row];
            let curr_row = &curr[offset..offset + bytes_per_row];

            total_diff += count_different_pixels(prev_row, curr_row, pixel_threshold);
        }

        total_diff
    }
}
