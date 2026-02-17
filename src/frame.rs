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

    /// Sample a pixel at (x, y) returning (R, G, B). Only works for CPU frames (RGBA layout).
    pub fn sample_pixel(&self, x: u32, y: u32) -> Option<(u8, u8, u8)> {
        match self {
            FrameData::Cpu { data, width, height } => {
                if x >= *width || y >= *height {
                    return None;
                }
                let idx = ((y * width + x) * 4) as usize;
                if idx + 2 < data.len() {
                    Some((data[idx], data[idx + 1], data[idx + 2]))
                } else {
                    None
                }
            }
            FrameData::DmaBuf { .. } => None,
        }
    }
}
