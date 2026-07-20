/// Rectangular changed area used by graphics damage planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DamageRegion {
    /// X coordinate of the region (pixels from left).
    pub x: u32,
    /// Y coordinate of the region (pixels from top).
    pub y: u32,
    /// Width of the region in pixels.
    pub width: u32,
    /// Height of the region in pixels.
    pub height: u32,
}

impl DamageRegion {
    #[inline]
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[inline]
    pub fn full_frame(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    #[inline]
    pub fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub fn overlaps(&self, other: &DamageRegion) -> bool {
        let self_right = self.x.saturating_add(self.width);
        let self_bottom = self.y.saturating_add(self.height);
        let other_right = other.x.saturating_add(other.width);
        let other_bottom = other.y.saturating_add(other.height);

        self.x < other_right
            && self_right > other.x
            && self.y < other_bottom
            && self_bottom > other.y
    }

    #[inline]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x
            && x < self.x.saturating_add(self.width)
            && y >= self.y
            && y < self.y.saturating_add(self.height)
    }

    pub fn union(&self, other: &DamageRegion) -> DamageRegion {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let bottom = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));

        DamageRegion {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }

    pub fn is_adjacent(&self, other: &DamageRegion, merge_distance: u32) -> bool {
        let self_right = self.x.saturating_add(self.width);
        let self_bottom = self.y.saturating_add(self.height);
        let other_right = other.x.saturating_add(other.width);
        let other_bottom = other.y.saturating_add(other.height);

        // Calculate horizontal gap (0 if overlapping)
        let gap_x = if other.x >= self_right {
            other.x - self_right
        } else {
            self.x.saturating_sub(other_right)
        };

        // Calculate vertical gap (0 if overlapping)
        let gap_y = if other.y >= self_bottom {
            other.y - self_bottom
        } else {
            self.y.saturating_sub(other_bottom)
        };

        // Adjacent if both gaps are within merge_distance
        gap_x <= merge_distance && gap_y <= merge_distance
    }
}
