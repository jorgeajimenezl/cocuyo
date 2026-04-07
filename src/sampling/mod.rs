mod average;
pub mod gpu;
mod max;
mod min;
mod palette;

pub use average::Average;
pub use max::Max;
pub use min::Min;
pub use palette::Palette;

use std::fmt;
use std::sync::Arc;

use crate::frame::FrameData;

/// Trait for sampling strategies that extract a single RGB color from a
/// rectangular region of BGRA pixel data.
///
/// Each implementation receives pre-computed, clamped region bounds and a
/// stride value that limits sampling to ~1000 pixels.
pub trait SamplingStrategy: Send + Sync + fmt::Debug {
    /// Unique identifier for this strategy (used for equality and serialization).
    fn id(&self) -> &'static str;

    /// Human-readable display name for the UI.
    fn name(&self) -> &'static str;

    /// Sample the pixel data within the given bounds.
    ///
    /// `data` is a contiguous BGRA buffer. The region spans rows `y0..y1` and
    /// columns `x0..x1` in a frame that is `width` pixels wide. `stride`
    /// controls the step between sampled pixels (both horizontally and
    /// vertically).
    fn sample(
        &self,
        data: &[u8],
        width: u32,
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
        stride: u32,
    ) -> Option<(u8, u8, u8)>;

    /// Whether this strategy supports GPU-accelerated sampling via compute shaders.
    fn supports_gpu(&self) -> bool {
        false
    }
}

/// Integer luminance: 299*R + 587*G + 114*B (scaled by 1000 vs the float
/// version). Avoids f64 conversions entirely. Range: 0..=255_000, fits in u32.
#[inline(always)]
pub(crate) fn luminance_u32(r: u8, g: u8, b: u8) -> u32 {
    299 * r as u32 + 587 * g as u32 + 114 * b as u32
}

/// Unified Max/Min sampling using a const generic bool.
/// `IS_MAX = true` → brightest pixel, `IS_MAX = false` → darkest pixel.
/// The const generic lets the compiler monomorphize two branchless versions.
pub(crate) fn sample_extremum<const IS_MAX: bool>(
    data: &[u8],
    width: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    stride: u32,
) -> Option<(u8, u8, u8)> {
    let mut best_rgb: (u8, u8, u8) = (0, 0, 0);
    let mut best_lum: u32 = if IS_MAX { 0 } else { u32::MAX };
    let mut found = false;

    for_each_sampled_pixel(data, width, x0, y0, x1, y1, stride, |r, g, b| {
        let lum = luminance_u32(r, g, b);
        let better = if IS_MAX {
            lum > best_lum
        } else {
            lum < best_lum
        };
        if better || !found {
            best_lum = lum;
            best_rgb = (r, g, b);
            found = true;
        }
    });

    if found { Some(best_rgb) } else { None }
}

/// Iterate over sampled pixels in a BGRA buffer within the given bounds,
/// stepping by `stride` in both dimensions. Calls `f(r, g, b)` for each pixel.
#[inline]
pub(crate) fn for_each_sampled_pixel(
    data: &[u8],
    width: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    stride: u32,
    mut f: impl FnMut(u8, u8, u8),
) {
    let mut py = y0;
    while py < y1 {
        let row_base = (py * width) as usize * 4;
        let mut px = x0;
        while px < x1 {
            let idx = row_base + (px as usize) * 4;
            if idx + 2 < data.len() {
                let b = data[idx];
                let g = data[idx + 1];
                let r = data[idx + 2];
                f(r, g, b);
            }
            px += stride;
        }
        py += stride;
    }
}

// ---------------------------------------------------------------------------
// BoxedStrategy – type-erased wrapper for iced pick_list compatibility
// ---------------------------------------------------------------------------

/// A type-erased sampling strategy providing `Clone`, `PartialEq`, `Eq`, and
/// `Display` so it can be used directly in iced `pick_list` widgets.
#[derive(Clone)]
pub struct BoxedStrategy(Arc<dyn SamplingStrategy>);

impl BoxedStrategy {
    pub fn new<S: SamplingStrategy + 'static>(strategy: S) -> Self {
        Self(Arc::new(strategy))
    }

    pub fn sample(
        &self,
        data: &[u8],
        width: u32,
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
        stride: u32,
    ) -> Option<(u8, u8, u8)> {
        self.0.sample(data, width, x0, y0, x1, y1, stride)
    }

    pub fn supports_gpu(&self) -> bool {
        self.0.supports_gpu()
    }

    pub fn id(&self) -> &'static str {
        self.0.id()
    }

    pub fn from_id(id: &str) -> Option<Self> {
        all_strategies().iter().find(|s| s.id() == id).cloned()
    }
}

impl fmt::Debug for BoxedStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq for BoxedStrategy {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl Eq for BoxedStrategy {}

impl fmt::Display for BoxedStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.name())
    }
}

impl Default for BoxedStrategy {
    fn default() -> Self {
        Self::new(Average)
    }
}

/// Returns all available sampling strategies (cached after first call).
///
/// To add a new strategy, create a type implementing [`SamplingStrategy`] and
/// append it here.
pub fn all_strategies() -> &'static [BoxedStrategy] {
    use std::sync::LazyLock;
    static ALL: LazyLock<Vec<BoxedStrategy>> = LazyLock::new(|| {
        vec![
            BoxedStrategy::new(Average),
            BoxedStrategy::new(Max),
            BoxedStrategy::new(Min),
            BoxedStrategy::new(Palette),
        ]
    });
    &ALL
}

// ---------------------------------------------------------------------------
// sample_region – shared bounds / stride logic
// ---------------------------------------------------------------------------

const TARGET_SAMPLE_PIXELS: f64 = 1000.0;

/// Clamp a floating-point region to pixel bounds within a frame.
/// Returns `None` if the clamped region is empty.
pub(crate) fn clamp_region_bounds(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    frame_w: u32,
    frame_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    let x0 = (x as u32).min(frame_w);
    let y0 = (y as u32).min(frame_h);
    let x1 = ((x + w) as u32).min(frame_w);
    let y1 = ((y + h) as u32).min(frame_h);
    if x0 >= x1 || y0 >= y1 {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

/// Sample a rectangular region of a frame using the given strategy.
///
/// Returns `None` if pixel data is unavailable or the region is empty/invalid.
/// Uses strided sampling (~1000 pixels max) to avoid processing every pixel.
pub fn sample_region(
    frame: &FrameData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    strategy: &BoxedStrategy,
) -> Option<(u8, u8, u8)> {
    let data = frame.pixels()?;
    let width = frame.width();
    let height = frame.height();

    let (x0, y0, x1, y1) = clamp_region_bounds(x, y, w, h, width, height)?;

    let region_w = x1 - x0;
    let region_h = y1 - y0;
    let total_pixels = (region_w as u64) * (region_h as u64);

    let stride = ((total_pixels as f64 / TARGET_SAMPLE_PIXELS).sqrt().ceil() as u32).max(1);

    strategy.sample(data, width, x0, y0, x1, y1, stride)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    /// Helper to create a CPU FrameData from raw BGRA bytes.
    fn cpu_frame(data: Vec<u8>, width: u32, height: u32) -> FrameData {
        FrameData::Cpu {
            data: Arc::new(data),
            width,
            height,
        }
    }

    /// 2x2 BGRA buffer:
    ///   (0,0) = red    (255,0,0)
    ///   (1,0) = green  (0,255,0)
    ///   (0,1) = blue   (0,0,255)
    ///   (1,1) = white  (255,255,255)
    fn make_2x2() -> FrameData {
        #[rustfmt::skip]
        let data = vec![
            0,   0,   255, 255,  // red   (B=0, G=0, R=255, A=255)
            0,   255, 0,   255,  // green (B=0, G=255, R=0, A=255)
            255, 0,   0,   255,  // blue  (B=255, G=0, R=0, A=255)
            255, 255, 255, 255,  // white
        ];
        cpu_frame(data, 2, 2)
    }

    fn average() -> BoxedStrategy {
        BoxedStrategy::new(Average)
    }

    fn max() -> BoxedStrategy {
        BoxedStrategy::new(Max)
    }

    fn min() -> BoxedStrategy {
        BoxedStrategy::new(Min)
    }

    #[test]
    fn average_2x2_full() {
        let frame = make_2x2();
        let result = sample_region(&frame, 0.0, 0.0, 2.0, 2.0, &average());
        // Average of (255,0,0), (0,255,0), (0,0,255), (255,255,255)
        // = (510/4, 510/4, 510/4) = (127, 127, 127)
        assert_eq!(result, Some((127, 127, 127)));
    }

    #[test]
    fn max_2x2_full() {
        let frame = make_2x2();
        let result = sample_region(&frame, 0.0, 0.0, 2.0, 2.0, &max());
        // White (255,255,255) has the highest luminance
        assert_eq!(result, Some((255, 255, 255)));
    }

    #[test]
    fn min_2x2_full() {
        let frame = make_2x2();
        let result = sample_region(&frame, 0.0, 0.0, 2.0, 2.0, &min());
        // Blue (0,0,255) has luminance 114*255 = 29070, the lowest
        assert_eq!(result, Some((0, 0, 255)));
    }

    #[test]
    fn zero_sized_region_returns_none() {
        let frame = make_2x2();
        assert_eq!(sample_region(&frame, 0.0, 0.0, 0.0, 0.0, &average()), None);
        assert_eq!(sample_region(&frame, 0.0, 0.0, 0.0, 1.0, &average()), None);
        assert_eq!(sample_region(&frame, 0.0, 0.0, 1.0, 0.0, &average()), None);
    }

    #[test]
    fn out_of_bounds_region_returns_none() {
        let frame = make_2x2();
        // Region entirely outside the frame
        assert_eq!(sample_region(&frame, 5.0, 5.0, 1.0, 1.0, &average()), None);
    }

    #[test]
    fn partially_out_of_bounds_region_clamps() {
        let frame = make_2x2();
        // Region extends beyond frame; should clamp to valid area
        let result = sample_region(&frame, 1.0, 0.0, 10.0, 10.0, &average());
        // Only pixels (1,0)=green and (1,1)=white are in bounds
        // Average: ((0+255)/2, (255+255)/2, (0+255)/2) = (127, 255, 127)
        assert_eq!(result, Some((127, 255, 127)));
    }

    #[test]
    fn single_pixel_last() {
        let frame = make_2x2();
        // Sample only the last pixel (1,1) = white
        let result = sample_region(&frame, 1.0, 1.0, 1.0, 1.0, &average());
        assert_eq!(result, Some((255, 255, 255)));
    }

    #[test]
    fn single_pixel_first() {
        let frame = make_2x2();
        // Sample only the first pixel (0,0) = red
        let result = sample_region(&frame, 0.0, 0.0, 1.0, 1.0, &average());
        assert_eq!(result, Some((255, 0, 0)));
    }

    fn palette() -> BoxedStrategy {
        BoxedStrategy::new(Palette)
    }

    #[test]
    fn palette_dominant_color() {
        // 4x4 frame: 12 red pixels + 4 blue pixels → palette returns red
        #[rustfmt::skip]
        let data = vec![
            // Row 0: red, red, red, blue (BGRA)
            0,0,255,255, 0,0,255,255, 0,0,255,255, 255,0,0,255,
            // Row 1: red, red, red, blue
            0,0,255,255, 0,0,255,255, 0,0,255,255, 255,0,0,255,
            // Row 2: red, red, red, blue
            0,0,255,255, 0,0,255,255, 0,0,255,255, 255,0,0,255,
            // Row 3: red, red, red, blue
            0,0,255,255, 0,0,255,255, 0,0,255,255, 255,0,0,255,
        ];
        let frame = cpu_frame(data, 4, 4);
        let result = sample_region(&frame, 0.0, 0.0, 4.0, 4.0, &palette());
        assert_eq!(result, Some((255, 0, 0)));
    }

    #[test]
    fn palette_single_color() {
        // All pixels same color → returns that color
        let frame = make_2x2();
        let result = sample_region(&frame, 0.0, 0.0, 1.0, 1.0, &palette());
        assert_eq!(result, Some((255, 0, 0)));
    }
}
