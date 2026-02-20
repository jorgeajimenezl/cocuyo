use std::os::fd::OwnedFd;
use std::sync::Arc;

use drm_fourcc::DrmFourcc;

impl std::fmt::Debug for FrameData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameData::DmaBuf { width, height, drm_format, rgba_pixels, .. } => {
                f.debug_struct("DmaBuf")
                    .field("width", width)
                    .field("height", height)
                    .field("drm_format", drm_format)
                    .field("has_rgba_pixels", &rgba_pixels.is_some())
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
        /// CPU-readable RGBA pixel data, populated when ambient sampling is active.
        rgba_pixels: Option<Arc<Vec<u8>>>,
    },
    Cpu {
        data: Arc<Vec<u8>>,
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

    /// Returns a reference to the RGBA pixel data, if available.
    ///
    /// - For `Cpu`: always returns `Some` (the GStreamer-converted RGBA data).
    /// - For `DmaBuf`: returns `Some` only when `rgba_pixels` was populated
    ///   (i.e., ambient sampling was active when the frame was captured).
    pub fn pixels(&self) -> Option<&[u8]> {
        match self {
            FrameData::Cpu { data, .. } => Some(data.as_slice()),
            FrameData::DmaBuf { rgba_pixels, .. } => rgba_pixels.as_ref().map(|d| d.as_slice()),
        }
    }

}
