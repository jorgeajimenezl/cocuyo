//! [`DmaBufFrame`] — the Linux zero-copy GPU frame type.
//!
//! Wraps a DMA-BUF file descriptor and its metadata. Implements [`GpuFrame`]
//! so the binary and sampling crate can import it as a wgpu texture without
//! knowing the Linux specifics.

use std::os::fd::{AsRawFd, OwnedFd};

use cocuyo_core::frame::{GpuFrame, ImportError};
use drm_fourcc::DrmFourcc;

/// A DMA-BUF backed frame produced by the PipeWire stream.
pub struct DmaBufFrame {
    pub fd: OwnedFd,
    pub width: u32,
    pub height: u32,
    pub drm_format: DrmFourcc,
    pub stride: u32,
    pub offset: u32,
    pub modifier: u64,
}

impl std::fmt::Debug for DmaBufFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DmaBufFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("drm_format", &self.drm_format)
            .finish()
    }
}

impl GpuFrame for DmaBufFrame {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn import_gpu(
        &self,
        device: &wgpu::Device,
    ) -> Result<(wgpu::Texture, wgpu::TextureFormat), ImportError> {
        unsafe {
            crate::vulkan_dmabuf::import_dmabuf_texture(
                device,
                self.fd.as_raw_fd(),
                self.width,
                self.height,
                self.drm_format,
                self.stride,
                self.offset,
            )
        }
        .map_err(|e| {
            crate::vulkan_dmabuf::mark_dmabuf_import_failed();
            ImportError::wrap(e)
        })
    }

    fn read_pixels_bgra(&self) -> Option<Vec<u8>> {
        crate::dmabuf_read::read_dmabuf_pixels(
            self.fd.as_raw_fd(),
            self.width,
            self.height,
            self.stride,
            self.offset,
            self.drm_format,
        )
        .map_err(|e| tracing::error!(error = %e, "DMA-BUF pixel readback failed"))
        .ok()
    }
}
