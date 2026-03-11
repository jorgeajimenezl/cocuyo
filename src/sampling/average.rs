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

        let mut py = y0;
        while py < y1 {
            let row_base = (py * width) as usize * 4;
            let mut px = x0;
            while px < x1 {
                let idx = row_base + (px as usize) * 4;
                if idx + 2 < data.len() {
                    b_sum += data[idx] as u64;
                    g_sum += data[idx + 1] as u64;
                    r_sum += data[idx + 2] as u64;
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
}
