use super::SamplingStrategy;

/// Finds the brightest pixel by luminance.
#[derive(Debug)]
pub struct Max;

impl SamplingStrategy for Max {
    fn id(&self) -> &'static str {
        "max"
    }

    fn name(&self) -> &'static str {
        "Max (brightest)"
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
        super::sample_extremum::<true>(data, width, x0, y0, x1, y1, stride)
    }
}
