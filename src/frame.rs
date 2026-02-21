use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

use drm_fourcc::DrmFourcc;

impl std::fmt::Debug for FrameData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameData::DmaBuf {
                width,
                height,
                drm_format,
                ..
            } => f
                .debug_struct("DmaBuf")
                .field("width", width)
                .field("height", height)
                .field("drm_format", drm_format)
                .finish(),
            FrameData::Cpu { width, height, .. } => f
                .debug_struct("Cpu")
                .field("width", width)
                .field("height", height)
                .finish(),
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
    pub fn pixels(&self) -> Option<&[u8]> {
        match self {
            FrameData::Cpu { data, .. } => Some(data.as_slice()),
            FrameData::DmaBuf { .. } => None,
        }
    }

    pub fn convert_to_cpu(self: &Arc<Self>) -> Option<Arc<FrameData>> {
        match self.as_ref() {
            FrameData::DmaBuf {
                fd,
                width,
                height,
                stride,
                offset,
                drm_format,
                ..
            } => {
                match crate::platform::linux::dmabuf_handler::read_dmabuf_pixels(
                    fd.as_raw_fd(),
                    *width,
                    *height,
                    *stride,
                    *offset,
                    *drm_format,
                ) {
                    Ok(rgba_data) => Some(Arc::new(FrameData::Cpu {
                        data: Arc::new(rgba_data),
                        width: *width,
                        height: *height,
                    })),
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to convert DmaBuf to RGBA");
                        None
                    }
                }
            }
            FrameData::Cpu { .. } => Some(self.clone()),
        }
    }
}
