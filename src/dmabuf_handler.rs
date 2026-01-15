use drm_fourcc::DrmFourcc;
use nix::sys::stat::fstat;
use pipewire as pw;
use std::os::fd::{RawFd, BorrowedFd};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DmaBufBuffer {
    pub fd: RawFd,
    pub width: u32,
    pub height: u32,
    pub format: DrmFourcc,
    pub stride: u32,
    pub offset: u32,
    pub modifier: u64,
}

#[derive(Debug)]
pub enum DmaBufError {
    NotAvailable(String),
    InvalidBuffer(String),
    UnsupportedFormat(String),
}

impl std::fmt::Display for DmaBufError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DmaBufError::NotAvailable(msg) => write!(f, "DMA-BUF not available: {}", msg),
            DmaBufError::InvalidBuffer(msg) => write!(f, "Invalid buffer: {}", msg),
            DmaBufError::UnsupportedFormat(msg) => write!(f, "Unsupported format: {}", msg),
        }
    }
}

impl std::error::Error for DmaBufError {}

impl DmaBufBuffer {
    /// Attempts to extract DMA-BUF information from a PipeWire buffer
    ///
    /// This function checks if the PipeWire buffer contains a DMA-BUF file descriptor
    /// and extracts the necessary metadata (fd, stride, offset, format modifier)
    ///
    /// Note: This requires mutable access to buffer to call datas_mut()
    pub fn from_pipewire_buffer(
        buffer: &mut pw::buffer::Buffer,
        width: u32,
        height: u32,
        format: pw::spa::param::video::VideoFormat,
    ) -> Result<Self, DmaBufError> {
        let datas = buffer.datas_mut();

        if datas.is_empty() {
            return Err(DmaBufError::InvalidBuffer(
                "Buffer has no data planes".to_string(),
            ));
        }

        let data = &datas[0];

        // Check if this is a DMA-BUF by examining the data type
        // In PipeWire, DMA-BUF buffers have a file descriptor instead of memory pointer
        let fd_i64 = match data.as_raw().fd {
            fd if fd >= 0 => fd,
            _ => {
                return Err(DmaBufError::NotAvailable(
                    "Buffer does not contain DMA-BUF file descriptor".to_string(),
                ));
            }
        };

        // Convert to i32 (RawFd)
        let fd: RawFd = fd_i64.try_into().map_err(|_| {
            DmaBufError::InvalidBuffer(format!("File descriptor {} out of range", fd_i64))
        })?;

        // Verify it's a valid file descriptor
        if let Err(e) = fstat(fd) {
            return Err(DmaBufError::InvalidBuffer(format!(
                "Invalid file descriptor: {}",
                e
            )));
        }

        // Get stride and offset from chunk metadata
        let chunk = data.chunk();
        let stride = chunk.stride() as u32;
        let offset = chunk.offset() as u32;

        // Convert PipeWire video format to DRM fourcc
        let drm_format = match format {
            pw::spa::param::video::VideoFormat::RGB => DrmFourcc::Rgb888,
            pw::spa::param::video::VideoFormat::RGBA => DrmFourcc::Abgr8888,
            pw::spa::param::video::VideoFormat::RGBx => DrmFourcc::Xbgr8888,
            pw::spa::param::video::VideoFormat::BGRx => DrmFourcc::Xrgb8888,
            pw::spa::param::video::VideoFormat::YUY2 => DrmFourcc::Yuyv,
            pw::spa::param::video::VideoFormat::I420 => DrmFourcc::Yuv420,
            _ => {
                return Err(DmaBufError::UnsupportedFormat(format!(
                    "Format {:?} not supported for DMA-BUF",
                    format
                )));
            }
        };

        // For now, assume no format modifiers (LINEAR layout)
        // In a full implementation, this should be queried from buffer metadata
        let modifier = drm_fourcc::DrmModifier::Linear.into();

        Ok(DmaBufBuffer {
            fd,
            width,
            height,
            format: drm_format,
            stride,
            offset,
            modifier,
        })
    }

    /// Check if DMA-BUF is likely available (heuristic check)
    #[allow(dead_code)]
    pub fn is_likely_available() -> bool {
        // Check if we're on a Wayland session (required for DMA-BUF)
        std::env::var("WAYLAND_DISPLAY").is_ok()
    }

    /// Maps the DMA-BUF to CPU-accessible memory
    ///
    /// This provides read-only access to the GPU buffer via mmap.
    /// While not true zero-copy, it avoids intermediate buffer allocations.
    ///
    /// # Safety
    /// The file descriptor must remain valid for the lifetime of the returned slice
    #[allow(dead_code)]
    pub unsafe fn map_readonly(&self) -> Result<Vec<u8>, DmaBufError> {
        use nix::sys::mman::{mmap, MapFlags, ProtFlags};

        let size = (self.stride * self.height) as usize;

        // Create a borrowed fd for the mmap call
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(self.fd) };

        let ptr = unsafe {
            mmap(
                None,
                std::num::NonZeroUsize::new(size)
                    .ok_or_else(|| DmaBufError::InvalidBuffer("Size is zero".to_string()))?,
                ProtFlags::PROT_READ,
                MapFlags::MAP_SHARED,
                borrowed_fd,
                self.offset as i64,
            )
            .map_err(|e| DmaBufError::InvalidBuffer(format!("mmap failed: {}", e)))?
        };

        // Copy the data to a Vec to avoid lifetime issues
        // The mmap will be automatically unmapped when we're done
        let data = unsafe { std::slice::from_raw_parts(ptr.as_ptr() as *const u8, size).to_vec() };

        // Unmap the memory
        unsafe {
            nix::sys::mman::munmap(ptr, size)
                .map_err(|e| DmaBufError::InvalidBuffer(format!("munmap failed: {}", e)))?
        };

        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dmabuf_availability_check() {
        // Just verify the function doesn't panic
        let _ = DmaBufBuffer::is_likely_available();
    }
}
