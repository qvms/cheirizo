use super::DamageRegion;

/// Merge adjacent/overlapping damage rectangles into coarser update regions.
pub(super) fn merge_regions(
    mut regions: Vec<DamageRegion>,
    merge_distance: u32,
) -> Vec<DamageRegion> {
    if regions.len() <= 1 {
        return regions;
    }

    let mut changed = true;
    while changed {
        changed = false;
        let mut merged = Vec::with_capacity(regions.len());
        let mut used = vec![false; regions.len()];

        for i in 0..regions.len() {
            if used[i] {
                continue;
            }

            let mut current = regions[i];
            used[i] = true;

            for j in (i + 1)..regions.len() {
                if used[j] {
                    continue;
                }

                if current.is_adjacent(&regions[j], merge_distance) {
                    current = current.union(&regions[j]);
                    used[j] = true;
                    changed = true;
                }
            }

            merged.push(current);
        }

        regions = merged;
    }

    regions
}

/// Convert dirty tile flags into bounded frame-relative damage regions.
pub(super) fn tiles_to_regions(
    dirty_tiles: &[bool],
    tiles_x: usize,
    tiles_y: usize,
    tile_size: usize,
    frame_width: u32,
    frame_height: u32,
) -> Vec<DamageRegion> {
    let mut regions = Vec::new();

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let idx = ty * tiles_x + tx;
            if dirty_tiles[idx] {
                let x = (tx * tile_size) as u32;
                let y = (ty * tile_size) as u32;
                let width = (tile_size as u32).min(frame_width.saturating_sub(x));
                let height = (tile_size as u32).min(frame_height.saturating_sub(y));

                if width > 0 && height > 0 {
                    regions.push(DamageRegion::new(x, y, width, height));
                }
            }
        }
    }

    regions
}
