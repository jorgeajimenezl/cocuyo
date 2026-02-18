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

    // Precompute the maximum safe byte index so we can hoist the bounds check
    // out of the inner loop. The last pixel we'd access is at (y1-1, x1-1),
    // needing idx+2. Any row/col within [y0..y1-stride, x0..x1-stride] with
    // stride alignment is guaranteed safe if max_safe_idx < data.len().
    let max_row = if y1 > 0 { y1 - 1 } else { 0 };
    let max_col = if x1 > 0 { x1 - 1 } else { 0 };
    let max_byte_idx = ((max_row * width + max_col) * 4 + 2) as usize;
    let bounds_safe = max_byte_idx < data.len();

    match strategy {
        SamplingStrategy::Average => sample_average(data, width, x0, y0, x1, y1, stride, bounds_safe),
        SamplingStrategy::Max => sample_extremum::<true>(data, width, x0, y0, x1, y1, stride, bounds_safe),
        SamplingStrategy::Min => sample_extremum::<false>(data, width, x0, y0, x1, y1, stride, bounds_safe),
    }
}

fn sample_average(
    data: &[u8],
    width: u32,
    x0: u32, y0: u32, x1: u32, y1: u32,
    stride: u32,
    bounds_safe: bool,
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
            if bounds_safe || idx + 2 < data.len() {
                // Safety of the indexing: when bounds_safe is true, all strided
                // pixels within [y0..y1, x0..x1] are guaranteed in-bounds
                // (checked via max_byte_idx above). When false, we check per-pixel.
                unsafe {
                    r_sum += *data.get_unchecked(idx) as u64;
                    g_sum += *data.get_unchecked(idx + 1) as u64;
                    b_sum += *data.get_unchecked(idx + 2) as u64;
                }
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
    bounds_safe: bool,
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
            if bounds_safe || idx + 2 < data.len() {
                let (r, g, b) = unsafe {
                    (
                        *data.get_unchecked(idx),
                        *data.get_unchecked(idx + 1),
                        *data.get_unchecked(idx + 2),
                    )
                };
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
