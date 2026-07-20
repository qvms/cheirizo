#![allow(unsafe_code)]

//! Pixel-difference primitives used by damage detection.

/// Count pixels that differ by more than the configured threshold (scalar path).
pub(super) fn count_different_pixels_scalar(prev: &[u8], curr: &[u8], threshold: u8) -> u32 {
    let mut count = 0u32;

    // Process 4 bytes at a time (BGRA pixels)
    for (p, c) in prev.chunks_exact(4).zip(curr.chunks_exact(4)) {
        // Check if any channel differs by more than threshold
        let diff_b = (p[0] as i16 - c[0] as i16).unsigned_abs() as u8;
        let diff_g = (p[1] as i16 - c[1] as i16).unsigned_abs() as u8;
        let diff_r = (p[2] as i16 - c[2] as i16).unsigned_abs() as u8;
        // Skip alpha channel (index 3)

        if diff_b > threshold || diff_g > threshold || diff_r > threshold {
            count += 1;
        }
    }

    count
}

#[inline]
pub(super) fn count_different_pixels(prev: &[u8], curr: &[u8], threshold: u8) -> u32 {
    // The former SIMD paths counted changed channels and divided by three,
    // which is not equivalent to counting pixels where any RGB channel changed.
    count_different_pixels_scalar(prev, curr, threshold)
}
