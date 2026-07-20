/// Configuration for graphics damage detection heuristics.
#[derive(Debug, Clone)]
pub struct DamageConfig {
    /// Size of each comparison tile in pixels.
    pub tile_size: usize,

    /// Fraction of tile pixels that must differ before a tile is marked dirty.
    pub diff_threshold: f32,

    /// Maximum per-channel pixel delta still treated as unchanged.
    pub pixel_threshold: u8,

    /// Distance threshold used when merging adjacent dirty tiles.
    pub merge_distance: u32,

    /// Minimum region area to emit; smaller regions are merged or ignored.
    pub min_region_area: u64,
}

impl Default for DamageConfig {
    fn default() -> Self {
        Self {
            tile_size: 64,
            diff_threshold: 0.05,
            pixel_threshold: 4,
            merge_distance: 32,
            min_region_area: 256,
        }
    }
}

impl DamageConfig {
    pub fn low_bandwidth() -> Self {
        Self {
            tile_size: 32,        // Finer granularity
            diff_threshold: 0.02, // More sensitive
            pixel_threshold: 2,
            merge_distance: 16,
            min_region_area: 64,
        }
    }

    pub fn high_motion() -> Self {
        Self {
            tile_size: 128,       // Coarser for speed
            diff_threshold: 0.10, // Less sensitive
            pixel_threshold: 8,
            merge_distance: 64,
            min_region_area: 1024,
        }
    }
}
