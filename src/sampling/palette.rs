use super::SamplingStrategy;
use super::gpu::HistogramBin;

const NUM_BINS: usize = 512;

/// Computes the dominant color using Fixed Histogram Quantization (8×8×8 bins).
#[derive(Debug)]
pub struct Palette;

impl Palette {
    pub const ID: &'static str = "palette";
}

#[derive(Clone, Copy, Default)]
struct Bin {
    r_sum: u64,
    g_sum: u64,
    b_sum: u64,
    count: u64,
}

#[inline(always)]
fn bin_index(r: u8, g: u8, b: u8) -> usize {
    ((r as usize >> 5) << 6) | ((g as usize >> 5) << 3) | (b as usize >> 5)
}

/// Extract the dominant color from a histogram of `HistogramBin` (u32 fields).
/// Shared between CPU readback of GPU results and could be reused elsewhere.
pub(super) fn extract_dominant_from_histogram(bins: &[HistogramBin]) -> Option<(u8, u8, u8)> {
    let best = bins.iter().max_by_key(|b| b.count)?;
    if best.count == 0 {
        return None;
    }
    Some((
        (best.r_sum / best.count) as u8,
        (best.g_sum / best.count) as u8,
        (best.b_sum / best.count) as u8,
    ))
}

impl SamplingStrategy for Palette {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn name(&self) -> &'static str {
        "Palette (dominant)"
    }

    fn supports_gpu(&self) -> bool {
        true
    }

    fn sample(
        &self,
        data: &[u8],
        width: u32,
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
        stride: u32,
    ) -> Option<(u8, u8, u8)> {
        let mut bins = [Bin::default(); NUM_BINS];

        super::for_each_sampled_pixel(data, width, x0, y0, x1, y1, stride, |r, g, b| {
            let bin = &mut bins[bin_index(r, g, b)];
            bin.r_sum += r as u64;
            bin.g_sum += g as u64;
            bin.b_sum += b as u64;
            bin.count += 1;
        });

        let best = bins.iter().max_by_key(|b| b.count)?;
        if best.count == 0 {
            return None;
        }

        Some((
            (best.r_sum / best.count) as u8,
            (best.g_sum / best.count) as u8,
            (best.b_sum / best.count) as u8,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::gpu::HistogramBin;

    #[test]
    fn extract_dominant_picks_highest_count() {
        let mut bins = vec![
            HistogramBin { r_sum: 0, g_sum: 0, b_sum: 0, count: 0 };
            NUM_BINS
        ];
        // Put 10 red-ish pixels in bin 0
        bins[0] = HistogramBin { r_sum: 2000, g_sum: 100, b_sum: 50, count: 10 };
        // Put 3 blue-ish pixels in bin 7
        bins[7] = HistogramBin { r_sum: 30, g_sum: 60, b_sum: 750, count: 3 };

        let (r, g, b) = extract_dominant_from_histogram(&bins).unwrap();
        // Should pick bin 0 (count=10): avg = (200, 10, 5)
        assert_eq!(r, 200);
        assert_eq!(g, 10);
        assert_eq!(b, 5);
    }

    #[test]
    fn extract_dominant_all_zero_returns_none() {
        let bins = vec![
            HistogramBin { r_sum: 0, g_sum: 0, b_sum: 0, count: 0 };
            NUM_BINS
        ];
        assert!(extract_dominant_from_histogram(&bins).is_none());
    }
}
