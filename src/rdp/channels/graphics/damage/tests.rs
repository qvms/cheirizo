//! Regression tests for graphics damage detection behavior.

use super::*;
use super::{
    compare::count_different_pixels_scalar,
    regions::{merge_regions, tiles_to_regions},
};

fn create_solid_frame(width: usize, height: usize, color: [u8; 4]) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 4];
    for pixel in data.chunks_exact_mut(4) {
        pixel.copy_from_slice(&color);
    }
    data
}

fn create_frame_with_region(
    width: usize,
    height: usize,
    bg_color: [u8; 4],
    region: DamageRegion,
    region_color: [u8; 4],
) -> Vec<u8> {
    let mut data = create_solid_frame(width, height, bg_color);

    for y in region.y..(region.y + region.height) {
        for x in region.x..(region.x + region.width) {
            if (x as usize) < width && (y as usize) < height {
                let idx = ((y as usize) * width + (x as usize)) * 4;
                data[idx..idx + 4].copy_from_slice(&region_color);
            }
        }
    }

    data
}

#[test]
fn test_damage_region_area() {
    let region = DamageRegion::new(0, 0, 100, 50);
    assert_eq!(region.area(), 5000);
}

#[test]
fn test_damage_region_full_frame() {
    let region = DamageRegion::full_frame(1920, 1080);
    assert_eq!(region.x, 0);
    assert_eq!(region.y, 0);
    assert_eq!(region.width, 1920);
    assert_eq!(region.height, 1080);
}

#[test]
fn test_damage_region_overlaps() {
    let r1 = DamageRegion::new(0, 0, 100, 100);
    let r2 = DamageRegion::new(50, 50, 100, 100);
    let r3 = DamageRegion::new(200, 200, 100, 100);

    assert!(r1.overlaps(&r2));
    assert!(r2.overlaps(&r1));
    assert!(!r1.overlaps(&r3));
    assert!(!r3.overlaps(&r1));
}

#[test]
fn test_damage_region_contains() {
    let region = DamageRegion::new(10, 20, 100, 50);
    assert!(region.contains(10, 20)); // Top-left
    assert!(region.contains(50, 40)); // Inside
    assert!(!region.contains(9, 20)); // Just outside left
    assert!(!region.contains(110, 20)); // Just outside right
}

#[test]
fn test_damage_region_union() {
    let r1 = DamageRegion::new(0, 0, 50, 50);
    let r2 = DamageRegion::new(30, 30, 50, 50);
    let union = r1.union(&r2);

    assert_eq!(union.x, 0);
    assert_eq!(union.y, 0);
    assert_eq!(union.width, 80);
    assert_eq!(union.height, 80);
}

#[test]
fn test_damage_region_is_adjacent() {
    let r1 = DamageRegion::new(0, 0, 64, 64);
    let r2 = DamageRegion::new(80, 0, 64, 64); // 16 pixels gap
    let r3 = DamageRegion::new(200, 0, 64, 64); // Far away

    assert!(r1.is_adjacent(&r2, 32)); // 32px merge distance covers gap
    assert!(!r1.is_adjacent(&r2, 10)); // 10px merge distance doesn't
    assert!(!r1.is_adjacent(&r3, 32));
}

#[test]
fn test_damage_config_default() {
    let config = DamageConfig::default();
    assert_eq!(config.tile_size, 64);
    assert!((config.diff_threshold - 0.05).abs() < 0.001);
    assert_eq!(config.pixel_threshold, 4);
    assert_eq!(config.merge_distance, 32);
}

#[test]
fn test_damage_config_presets() {
    let low_bw = DamageConfig::low_bandwidth();
    assert_eq!(low_bw.tile_size, 32);
    assert!(low_bw.diff_threshold < 0.05);

    let high_motion = DamageConfig::high_motion();
    assert_eq!(high_motion.tile_size, 128);
    assert!(high_motion.diff_threshold > 0.05);
}

#[test]
fn test_count_different_pixels_identical() {
    let data = vec![100u8; 64];
    let count = count_different_pixels_scalar(&data, &data, 4);
    assert_eq!(count, 0);
}

#[test]
fn test_count_different_pixels_all_different() {
    let prev = vec![0u8; 64];
    let curr = vec![255u8; 64];
    let count = count_different_pixels_scalar(&prev, &curr, 4);
    assert_eq!(count, 16); // 64 bytes / 4 bytes per pixel
}

#[test]
fn test_count_different_pixels_threshold() {
    let prev = vec![100u8; 64];
    let mut curr = prev.clone();

    // Change first pixel slightly (within threshold)
    curr[0] = 103; // Diff of 3
    let count = count_different_pixels_scalar(&prev, &curr, 4);
    assert_eq!(count, 0); // Below threshold

    // Change first pixel more (exceeds threshold)
    curr[0] = 110; // Diff of 10
    let count = count_different_pixels_scalar(&prev, &curr, 4);
    assert_eq!(count, 1); // Above threshold
}

#[test]
fn test_merge_regions_empty() {
    let regions = merge_regions(vec![], 32);
    assert!(regions.is_empty());
}

#[test]
fn test_merge_regions_single() {
    let region = DamageRegion::new(0, 0, 64, 64);
    let regions = merge_regions(vec![region], 32);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0], region);
}

#[test]
fn test_merge_regions_adjacent() {
    let r1 = DamageRegion::new(0, 0, 64, 64);
    let r2 = DamageRegion::new(64, 0, 64, 64); // Adjacent

    let regions = merge_regions(vec![r1, r2], 32);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].width, 128);
}

#[test]
fn test_merge_regions_separate() {
    let r1 = DamageRegion::new(0, 0, 64, 64);
    let r2 = DamageRegion::new(200, 200, 64, 64); // Far apart

    let regions = merge_regions(vec![r1, r2], 32);
    assert_eq!(regions.len(), 2);
}

#[test]
fn test_merge_regions_chain() {
    // Three regions in a chain: A-B-C where A adjacent to B, B adjacent to C
    let r1 = DamageRegion::new(0, 0, 64, 64);
    let r2 = DamageRegion::new(80, 0, 64, 64);
    let r3 = DamageRegion::new(160, 0, 64, 64);

    let regions = merge_regions(vec![r1, r2, r3], 32);
    assert_eq!(regions.len(), 1); // All merged into one
    assert_eq!(regions[0].width, 224);
}

#[test]
fn test_detector_first_frame_full_damage() {
    let mut detector = DamageDetector::with_defaults();
    let frame = create_solid_frame(640, 480, [0, 0, 0, 255]);

    let damage = detector.detect(&frame, 640, 480);

    assert_eq!(damage.len(), 1);
    assert_eq!(damage[0], DamageRegion::full_frame(640, 480));
}

#[test]
fn test_detector_identical_frames_no_damage() {
    let mut detector = DamageDetector::with_defaults();
    let frame = create_solid_frame(640, 480, [100, 100, 100, 255]);

    // First frame
    let _ = detector.detect(&frame, 640, 480);

    // Second identical frame
    let damage = detector.detect(&frame, 640, 480);
    assert!(damage.is_empty(), "Identical frames should have no damage");
}

#[test]
fn test_detector_partial_change() {
    let mut detector = DamageDetector::new(DamageConfig {
        tile_size: 64,
        diff_threshold: 0.01, // Very sensitive
        pixel_threshold: 1,
        merge_distance: 0, // No merging
        min_region_area: 1,
    });

    let frame1 = create_solid_frame(256, 256, [0, 0, 0, 255]);

    // Create frame with a changed region in top-left corner
    let changed_region = DamageRegion::new(0, 0, 64, 64);
    let frame2 = create_frame_with_region(
        256,
        256,
        [0, 0, 0, 255],
        changed_region,
        [255, 255, 255, 255],
    );

    // First frame
    let _ = detector.detect(&frame1, 256, 256);

    // Second frame with partial change
    let damage = detector.detect(&frame2, 256, 256);

    assert!(!damage.is_empty(), "Should detect damage");

    // Check that damage is in the expected area
    let total_damage_area: u64 = damage.iter().map(super::DamageRegion::area).sum();
    let expected_area = changed_region.area();
    assert!(
        total_damage_area >= expected_area / 2,
        "Damage area {total_damage_area} should include changed region {expected_area}"
    );
}

#[test]
fn test_detector_dimension_change_invalidates() {
    let mut detector = DamageDetector::with_defaults();

    let frame1 = create_solid_frame(640, 480, [100, 100, 100, 255]);
    let frame2 = create_solid_frame(800, 600, [100, 100, 100, 255]);

    // First frame at 640x480
    let damage1 = detector.detect(&frame1, 640, 480);
    assert_eq!(damage1[0], DamageRegion::full_frame(640, 480));

    // Second frame at different resolution
    let damage2 = detector.detect(&frame2, 800, 600);
    assert_eq!(damage2.len(), 1);
    assert_eq!(damage2[0], DamageRegion::full_frame(800, 600));
}

#[test]
fn test_detector_invalidate() {
    let mut detector = DamageDetector::with_defaults();
    let frame = create_solid_frame(640, 480, [100, 100, 100, 255]);

    // First frame
    let _ = detector.detect(&frame, 640, 480);

    // Invalidate
    detector.invalidate();

    // Should get full damage again
    let damage = detector.detect(&frame, 640, 480);
    assert_eq!(damage.len(), 1);
    assert_eq!(damage[0], DamageRegion::full_frame(640, 480));
}

#[test]
fn test_detector_stats() {
    let mut detector = DamageDetector::with_defaults();
    let frame = create_solid_frame(640, 480, [0, 0, 0, 255]);

    // Process several frames
    for _ in 0..5 {
        let _ = detector.detect(&frame, 640, 480);
    }

    let stats = detector.stats();
    assert_eq!(stats.frames_processed, 5);
    assert_eq!(stats.frames_full, 1); // Initial full-frame detection
    assert_eq!(stats.frames_skipped, 4); // Subsequent unchanged frames
    assert!(stats.bandwidth_reduction_percent() > 0.0);
}

#[test]
fn test_detector_config_update() {
    let mut detector = DamageDetector::with_defaults();
    let frame = create_solid_frame(640, 480, [100, 100, 100, 255]);

    // First frame
    let _ = detector.detect(&frame, 640, 480);

    // Update config
    detector.set_config(DamageConfig::high_motion());

    // Should invalidate and return full damage
    let damage = detector.detect(&frame, 640, 480);
    assert_eq!(damage.len(), 1);
    assert_eq!(damage[0], DamageRegion::full_frame(640, 480));
}

#[test]
fn test_detector_odd_dimensions() {
    let mut detector = DamageDetector::with_defaults();
    let frame = create_solid_frame(641, 479, [128, 128, 128, 255]); // Odd dimensions

    let damage = detector.detect(&frame, 641, 479);
    assert_eq!(damage.len(), 1);
    assert_eq!(damage[0], DamageRegion::full_frame(641, 479));

    // Second frame should work too
    let damage2 = detector.detect(&frame, 641, 479);
    assert!(damage2.is_empty());
}

#[test]
fn test_detector_small_frame() {
    let mut detector = DamageDetector::new(DamageConfig {
        tile_size: 64,
        min_region_area: 1,
        ..Default::default()
    });
    let frame = create_solid_frame(32, 32, [50, 50, 50, 255]); // Smaller than tile

    let damage = detector.detect(&frame, 32, 32);
    assert_eq!(damage.len(), 1);
    assert_eq!(damage[0].area(), 32 * 32);
}

#[test]
fn test_detector_large_frame() {
    let mut detector = DamageDetector::with_defaults();
    let frame = create_solid_frame(3840, 2160, [0, 128, 255, 255]); // 4K

    let damage = detector.detect(&frame, 3840, 2160);
    assert_eq!(damage.len(), 1);
    assert_eq!(damage[0], DamageRegion::full_frame(3840, 2160));

    // Identical second frame
    let damage2 = detector.detect(&frame, 3840, 2160);
    assert!(damage2.is_empty());
}

#[test]
#[should_panic(expected = "Frame size mismatch")]
fn test_detector_wrong_size_panics() {
    let mut detector = DamageDetector::with_defaults();
    let frame = create_solid_frame(640, 480, [0, 0, 0, 255]);

    // Pass wrong dimensions
    let _ = detector.detect(&frame, 800, 600);
}

#[test]
fn test_tiles_to_regions_single() {
    let mut dirty = vec![false; 16]; // 4×4 grid
    dirty[5] = true; // (1, 1) tile

    let regions = tiles_to_regions(&dirty, 4, 4, 64, 256, 256);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].x, 64);
    assert_eq!(regions[0].y, 64);
    assert_eq!(regions[0].width, 64);
    assert_eq!(regions[0].height, 64);
}

#[test]
fn test_tiles_to_regions_edge_clamping() {
    let mut dirty = vec![false; 4]; // 2×2 grid
    dirty[3] = true; // Bottom-right tile

    // Frame is 100×100 with 64px tiles
    // Bottom-right tile should be clamped to (64, 64, 36, 36)
    let regions = tiles_to_regions(&dirty, 2, 2, 64, 100, 100);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].x, 64);
    assert_eq!(regions[0].y, 64);
    assert_eq!(regions[0].width, 36);
    assert_eq!(regions[0].height, 36);
}
