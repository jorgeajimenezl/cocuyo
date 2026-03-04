use super::{SamplingStrategy, luminance_u32};

/// Finds the darkest pixel by luminance.
#[derive(Debug)]
pub struct Min;

impl SamplingStrategy for Min {
    fn id(&self) -> &'static str {
        "min"
    }

    fn name(&self) -> &'static str {
        "Min (darkest)"
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
        let mut best_rgb: (u8, u8, u8) = (0, 0, 0);
        let mut best_lum: u32 = u32::MAX;
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
                    if lum < best_lum || !found {
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
}
