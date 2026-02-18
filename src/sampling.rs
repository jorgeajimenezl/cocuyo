use crate::frame::FrameData;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SamplingStrategy {
    #[default]
    Average,
    Max,
    Min,
}

impl SamplingStrategy {
    pub const ALL: &'static [SamplingStrategy] = &[
        SamplingStrategy::Average,
        SamplingStrategy::Max,
        SamplingStrategy::Min,
    ];
}

impl std::fmt::Display for SamplingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SamplingStrategy::Average => write!(f, "Average"),
            SamplingStrategy::Max => write!(f, "Max (brightest)"),
            SamplingStrategy::Min => write!(f, "Min (darkest)"),
        }
    }
}

/// Integer luminance: 299*R + 587*G + 114*B (scaled by 1000 vs the float version).
/// Avoids f64 conversions entirely. Range: 0..=255_000, fits in u32.
#[inline(always)]
fn luminance_u32(r: u8, g: u8, b: u8) -> u32 {
    299 * r as u32 + 587 * g as u32 + 114 * b as u32
}

/// Sample a rectangular region of a CPU frame using the given strategy.
///
/// Returns `None` for DMA-BUF frames or empty/invalid regions.
/// Uses strided sampling (~1000 pixels max) to avoid processing every pixel.
pub fn sample_region(
    frame: &FrameData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    strategy: SamplingStrategy,
) -> Option<(u8, u8, u8)> {
    let FrameData::Cpu { data, width, height } = frame else {
        return None;
    };

    let width = *width;
    let height = *height;

    let x0 = (x as u32).min(width);
    let y0 = (y as u32).min(height);
    let x1 = ((x + w) as u32).min(width);
    let y1 = ((y + h) as u32).min(height);

    if x0 >= x1 || y0 >= y1 {
        return None;
    }

    let region_w = x1 - x0;
    let region_h = y1 - y0;
    let total_pixels = (region_w as u64) * (region_h as u64);

    // Determine stride to sample ~1000 pixels max
    let stride = ((total_pixels as f64 / 1000.0).sqrt().ceil() as u32).max(1);

    match strategy {
        SamplingStrategy::Average => sample_average(data, width, x0, y0, x1, y1, stride),
        SamplingStrategy::Max => sample_extremum::<true>(data, width, x0, y0, x1, y1, stride),
        SamplingStrategy::Min => sample_extremum::<false>(data, width, x0, y0, x1, y1, stride),
    }
}

fn sample_average(
    data: &[u8],
    width: u32,
    x0: u32, y0: u32, x1: u32, y1: u32,
    stride: u32,
) -> Option<(u8, u8, u8)> {
    let mut r_sum: u64 = 0;
    let mut g_sum: u64 = 0;
    let mut b_sum: u64 = 0;
    let mut count: u64 = 0;

    let mut py = y0;
    while py < y1 {
        let row_base = (py * width) as usize * 4;
        let mut px = x0;
        while px < x1 {
            let idx = row_base + (px as usize) * 4;
            if idx + 2 < data.len() {
                r_sum += data[idx] as u64;
                g_sum += data[idx + 1] as u64;
                b_sum += data[idx + 2] as u64;
                count += 1;
            }
            px += stride;
        }
        py += stride;
    }

    if count == 0 {
        return None;
    }

    Some((
        (r_sum / count) as u8,
        (g_sum / count) as u8,
        (b_sum / count) as u8,
    ))
}

/// Unified Max/Min sampling using a const generic bool.
/// `IS_MAX = true` → brightest pixel, `IS_MAX = false` → darkest pixel.
/// The const generic lets the compiler monomorphize two branchless versions.
fn sample_extremum<const IS_MAX: bool>(
    data: &[u8],
    width: u32,
    x0: u32, y0: u32, x1: u32, y1: u32,
    stride: u32,
) -> Option<(u8, u8, u8)> {
    let mut best_rgb: (u8, u8, u8) = (0, 0, 0);
    let mut best_lum: u32 = if IS_MAX { 0 } else { u32::MAX };
    let mut found = false;

    let mut py = y0;
    while py < y1 {
        let row_base = (py * width) as usize * 4;
        let mut px = x0;
        while px < x1 {
            let idx = row_base + (px as usize) * 4;
            if idx + 2 < data.len() {
                let (r, g, b) = (data[idx], data[idx + 1], data[idx + 2]);
                let lum = luminance_u32(r, g, b);
                let better = if IS_MAX { lum > best_lum } else { lum < best_lum };
                if better || !found {
                    best_lum = lum;
                    best_rgb = (r, g, b);
                    found = true;
                }
            }
            px += stride;
        }
        py += stride;
    }

    if found { Some(best_rgb) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    /// Helper to create a CPU FrameData from raw RGBA bytes.
    fn cpu_frame(data: Vec<u8>, width: u32, height: u32) -> FrameData {
        FrameData::Cpu { data: Arc::new(data), width, height }
    }

    /// 2×2 RGBA buffer:
    ///   (0,0) = red    (255,0,0)
    ///   (1,0) = green  (0,255,0)
    ///   (0,1) = blue   (0,0,255)
    ///   (1,1) = white  (255,255,255)
    fn make_2x2() -> FrameData {
        #[rustfmt::skip]
        let data = vec![
            255, 0,   0,   255,  // red
            0,   255, 0,   255,  // green
            0,   0,   255, 255,  // blue
            255, 255, 255, 255,  // white
        ];
        cpu_frame(data, 2, 2)
    }

    #[test]
    fn average_2x2_full() {
        let frame = make_2x2();
        let result = sample_region(&frame, 0.0, 0.0, 2.0, 2.0, SamplingStrategy::Average);
        // Average of (255,0,0), (0,255,0), (0,0,255), (255,255,255)
        // = (510/4, 510/4, 510/4) = (127, 127, 127)
        assert_eq!(result, Some((127, 127, 127)));
    }

    #[test]
    fn max_2x2_full() {
        let frame = make_2x2();
        let result = sample_region(&frame, 0.0, 0.0, 2.0, 2.0, SamplingStrategy::Max);
        // White (255,255,255) has the highest luminance
        assert_eq!(result, Some((255, 255, 255)));
    }

    #[test]
    fn min_2x2_full() {
        let frame = make_2x2();
        let result = sample_region(&frame, 0.0, 0.0, 2.0, 2.0, SamplingStrategy::Min);
        // Blue (0,0,255) has luminance 114*255 = 29070, the lowest
        assert_eq!(result, Some((0, 0, 255)));
    }

    #[test]
    fn zero_sized_region_returns_none() {
        let frame = make_2x2();
        assert_eq!(sample_region(&frame, 0.0, 0.0, 0.0, 0.0, SamplingStrategy::Average), None);
        assert_eq!(sample_region(&frame, 0.0, 0.0, 0.0, 1.0, SamplingStrategy::Average), None);
        assert_eq!(sample_region(&frame, 0.0, 0.0, 1.0, 0.0, SamplingStrategy::Average), None);
    }

    #[test]
    fn out_of_bounds_region_returns_none() {
        let frame = make_2x2();
        // Region entirely outside the frame
        assert_eq!(sample_region(&frame, 5.0, 5.0, 1.0, 1.0, SamplingStrategy::Average), None);
    }

    #[test]
    fn partially_out_of_bounds_region_clamps() {
        let frame = make_2x2();
        // Region extends beyond frame; should clamp to valid area
        let result = sample_region(&frame, 1.0, 0.0, 10.0, 10.0, SamplingStrategy::Average);
        // Only pixels (1,0)=green and (1,1)=white are in bounds
        // Average: ((0+255)/2, (255+255)/2, (0+255)/2) = (127, 255, 127)
        assert_eq!(result, Some((127, 255, 127)));
    }

    #[test]
    fn single_pixel_last() {
        let frame = make_2x2();
        // Sample only the last pixel (1,1) = white
        let result = sample_region(&frame, 1.0, 1.0, 1.0, 1.0, SamplingStrategy::Average);
        assert_eq!(result, Some((255, 255, 255)));
    }

    #[test]
    fn single_pixel_first() {
        let frame = make_2x2();
        // Sample only the first pixel (0,0) = red
        let result = sample_region(&frame, 0.0, 0.0, 1.0, 1.0, SamplingStrategy::Average);
        assert_eq!(result, Some((255, 0, 0)));
    }
}
