use std::os::fd::OwnedFd;

use drm_fourcc::DrmFourcc;

impl std::fmt::Debug for FrameData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameData::DmaBuf { width, height, drm_format, .. } => {
                f.debug_struct("DmaBuf")
                    .field("width", width)
                    .field("height", height)
                    .field("drm_format", drm_format)
                    .finish()
            }
            FrameData::Cpu { width, height, .. } => {
                f.debug_struct("Cpu")
                    .field("width", width)
                    .field("height", height)
                    .finish()
            }
        }
    }
}

pub enum FrameData {
    DmaBuf {
        fd: OwnedFd,
        width: u32,
        height: u32,
        drm_format: DrmFourcc,
        stride: u32,
        #[allow(dead_code)]
        offset: u32,
        #[allow(dead_code)]
        modifier: u64,
    },
    Cpu {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
}

impl FrameData {
    pub fn width(&self) -> u32 {
        match self {
            FrameData::DmaBuf { width, .. } => *width,
            FrameData::Cpu { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            FrameData::DmaBuf { height, .. } => *height,
            FrameData::Cpu { height, .. } => *height,
        }
    }

    /// Sample the average color of a rectangular region. Only works for CPU frames.
    /// Uses strided sampling (~1000 pixels max) to avoid processing every pixel.
    pub fn sample_region_average(&self, x: f32, y: f32, w: f32, h: f32) -> Option<(u8, u8, u8)> {
        let FrameData::Cpu { data, width, height } = self else {
            return None;
        };

        let x0 = (x as u32).min(*width);
        let y0 = (y as u32).min(*height);
        let x1 = ((x + w) as u32).min(*width);
        let y1 = ((y + h) as u32).min(*height);

        if x0 >= x1 || y0 >= y1 {
            return None;
        }

        let region_w = x1 - x0;
        let region_h = y1 - y0;
        let total_pixels = (region_w as u64) * (region_h as u64);

        // Determine stride to sample ~1000 pixels max
        let stride = ((total_pixels as f64 / 1000.0).sqrt().ceil() as u32).max(1);

        let mut r_sum: u64 = 0;
        let mut g_sum: u64 = 0;
        let mut b_sum: u64 = 0;
        let mut count: u64 = 0;

        let mut py = y0;
        while py < y1 {
            let mut px = x0;
            while px < x1 {
                let idx = ((py * width + px) * 4) as usize;
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
}
