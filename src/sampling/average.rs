use super::SamplingStrategy;

/// Computes the mean R, G, B across sampled pixels.
#[derive(Debug)]
pub struct Average;

impl SamplingStrategy for Average {
    fn id(&self) -> &'static str {
        "average"
    }

    fn name(&self) -> &'static str {
        "Average"
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
        let mut r_sum: u64 = 0;
        let mut g_sum: u64 = 0;
        let mut b_sum: u64 = 0;
        let mut count: u64 = 0;

        super::for_each_sampled_pixel(data, width, x0, y0, x1, y1, stride, |r, g, b| {
            r_sum += r as u64;
            g_sum += g as u64;
            b_sum += b as u64;
            count += 1;
        });

        if count == 0 {
            return None;
        }

        Some((
            (r_sum / count) as u8,
            (g_sum / count) as u8,
            (b_sum / count) as u8,
        ))
    }
}
