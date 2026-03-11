#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

#[cfg(target_os = "linux")]
use drm_fourcc::DrmFourcc;

#[cfg(target_os = "macos")]
use screencapturekit::cm::IOSurface;

#[cfg(target_os = "windows")]
use crate::platform::windows::shared_texture;

impl std::fmt::Debug for FrameData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(target_os = "linux")]
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
            #[cfg(target_os = "macos")]
            FrameData::IOSurface { width, height, .. } => f
                .debug_struct("IOSurface")
                .field("width", width)
                .field("height", height)
                .finish(),
            #[cfg(target_os = "windows")]
            FrameData::D3DShared { width, height, .. } => f
                .debug_struct("D3DShared")
                .field("width", width)
                .field("height", height)
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "macos")]
    IOSurface {
        surface: IOSurface,
        width: u32,
        height: u32,
    },
    #[cfg(target_os = "windows")]
    D3DShared {
        frame: shared_texture::HeldFrame,
        width: u32,
        height: u32,
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
            #[cfg(target_os = "linux")]
            FrameData::DmaBuf { width, .. } => *width,
            #[cfg(target_os = "macos")]
            FrameData::IOSurface { width, .. } => *width,
            #[cfg(target_os = "windows")]
            FrameData::D3DShared { width, .. } => *width,
            FrameData::Cpu { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            #[cfg(target_os = "linux")]
            FrameData::DmaBuf { height, .. } => *height,
            #[cfg(target_os = "macos")]
            FrameData::IOSurface { height, .. } => *height,
            #[cfg(target_os = "windows")]
            FrameData::D3DShared { height, .. } => *height,
            FrameData::Cpu { height, .. } => *height,
        }
    }

    /// Returns a reference to the RGBA pixel data, if available.
    pub fn pixels(&self) -> Option<&[u8]> {
        match self {
            FrameData::Cpu { data, .. } => Some(data.as_slice()),
            #[cfg(target_os = "linux")]
            FrameData::DmaBuf { .. } => None,
            #[cfg(target_os = "macos")]
            FrameData::IOSurface { .. } => None,
            #[cfg(target_os = "windows")]
            FrameData::D3DShared { .. } => None,
        }
    }

    pub fn convert_to_cpu(self: &Arc<Self>) -> Option<Arc<FrameData>> {
        match self.as_ref() {
            #[cfg(target_os = "linux")]
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
            #[cfg(target_os = "macos")]
            FrameData::IOSurface {
                surface,
                width,
                height,
            } => {
                match surface.lock_read_only() {
                    Ok(guard) => {
                        let bpr = surface.bytes_per_row();
                        let src = guard.as_slice();
                        let rgba = crate::platform::macos::recording::bgra_to_rgba(
                            src,
                            *width as usize,
                            *height as usize,
                            bpr,
                        );
                        Some(Arc::new(FrameData::Cpu {
                            data: Arc::new(rgba),
                            width: *width,
                            height: *height,
                        }))
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to lock IOSurface for CPU readback");
                        None
                    }
                }
            }
            #[cfg(target_os = "windows")]
            FrameData::D3DShared {
                frame,
                width,
                height,
            } => match frame.read_pixels() {
                Ok(rgba_data) => Some(Arc::new(FrameData::Cpu {
                    data: Arc::new(rgba_data),
                    width: *width,
                    height: *height,
                })),
                Err(e) => {
                    tracing::error!(error = %e, "Failed to read shared texture pixels");
                    None
                }
            },
            FrameData::Cpu { .. } => Some(self.clone()),
        }
    }
}
