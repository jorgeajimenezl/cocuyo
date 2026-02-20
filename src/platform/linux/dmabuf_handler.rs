use nix::sys::stat::fstat;
use pipewire as pw;
use std::num::NonZeroUsize;
use std::os::fd::{BorrowedFd, RawFd};

use super::formats::to_drm_format;

// DMA-BUF sync ioctl constants
const DMA_BUF_SYNC_READ: u64 = 1 << 0;
const DMA_BUF_SYNC_START: u64 = 0 << 2;
const DMA_BUF_SYNC_END: u64 = 1 << 2;

nix::ioctl_write_ptr!(dma_buf_ioctl_sync, b'b', 0, u64);

#[derive(Debug, Clone)]
pub struct DmaBufBuffer {
    pub fd: RawFd,
    pub width: u32,
    pub height: u32,
    pub format: drm_fourcc::DrmFourcc,
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
    /// Extracts DMA-BUF information from a PipeWire buffer.
    pub fn from_pipewire_buffer(
        buffer: &mut pw::buffer::Buffer,
        width: u32,
        height: u32,
        format: pw::spa::param::video::VideoFormat,
        modifier: u64,
    ) -> Result<Self, DmaBufError> {
        let datas = buffer.datas_mut();

        if datas.is_empty() {
            return Err(DmaBufError::InvalidBuffer(
                "Buffer has no data planes".to_string(),
            ));
        }

        let data = &datas[0];

        let fd_i64 = match data.as_raw().fd {
            fd if fd >= 0 => fd,
            _ => {
                return Err(DmaBufError::NotAvailable(
                    "Buffer does not contain DMA-BUF file descriptor".to_string(),
                ));
            }
        };

        let fd: RawFd = fd_i64.try_into().map_err(|_| {
            DmaBufError::InvalidBuffer(format!("File descriptor {} out of range", fd_i64))
        })?;

        if let Err(e) = fstat(fd) {
            return Err(DmaBufError::InvalidBuffer(format!(
                "Invalid file descriptor: {}",
                e
            )));
        }

        let chunk = data.chunk();
        let stride = chunk.stride() as u32;
        let offset = chunk.offset() as u32;

        let drm_format = to_drm_format(format).ok_or_else(|| {
            DmaBufError::UnsupportedFormat(format!("Format {:?} not supported for DMA-BUF", format))
        })?;

        // Use the real modifier from PipeWire's negotiated format.
        // A value of 0 means DRM_FORMAT_MOD_LINEAR.

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
}

#[derive(Debug)]
pub enum DmaBufReadError {
    MmapFailed(nix::errno::Errno),
    SyncFailed(nix::errno::Errno),
    MunmapFailed(nix::errno::Errno),
    UnsupportedFormat(drm_fourcc::DrmFourcc),
    InvalidBufferSize,
}

impl std::fmt::Display for DmaBufReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DmaBufReadError::MmapFailed(e) => write!(f, "DMA-BUF mmap failed: {}", e),
            DmaBufReadError::SyncFailed(e) => write!(f, "DMA-BUF sync ioctl failed: {}", e),
            DmaBufReadError::MunmapFailed(e) => write!(f, "DMA-BUF munmap failed: {}", e),
            DmaBufReadError::UnsupportedFormat(fmt) => {
                write!(f, "Unsupported DRM format for pixel read: {:?}", fmt)
            }
            DmaBufReadError::InvalidBufferSize => write!(f, "DMA-BUF buffer size is zero"),
        }
    }
}

impl std::error::Error for DmaBufReadError {}

/// Reads pixel data from a DMA-BUF fd via mmap, returning tightly-packed RGBA bytes.
///
/// Handles stride padding (stride may be > width * 4) and normalizes BGRA
/// formats (Xrgb8888, Argb8888) to RGBA byte order.
pub fn read_dmabuf_pixels(
    fd: RawFd,
    width: u32,
    height: u32,
    stride: u32,
    offset: u32,
    format: drm_fourcc::DrmFourcc,
) -> Result<Vec<u8>, DmaBufReadError> {
    use drm_fourcc::DrmFourcc;
    use nix::sys::mman::{mmap, munmap, MapFlags, ProtFlags};

    let needs_bgra_swap = matches!(format, DrmFourcc::Xrgb8888 | DrmFourcc::Argb8888);
    let has_padding_alpha = matches!(format, DrmFourcc::Xrgb8888 | DrmFourcc::Xbgr8888);

    if !matches!(
        format,
        DrmFourcc::Xrgb8888 | DrmFourcc::Argb8888 | DrmFourcc::Abgr8888 | DrmFourcc::Xbgr8888
    ) {
        return Err(DmaBufReadError::UnsupportedFormat(format));
    }

    let row_bytes = (width as usize) * 4;
    let map_size = offset as usize + (stride as usize) * (height as usize);

    let map_len =
        NonZeroUsize::new(map_size).ok_or(DmaBufReadError::InvalidBufferSize)?;

    // SAFETY: fd is a valid DMA-BUF fd during the PipeWire on_process callback.
    let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
    let mapped_ptr = unsafe {
        mmap(
            None,
            map_len,
            ProtFlags::PROT_READ,
            MapFlags::MAP_SHARED,
            borrowed_fd,
            0, // DMA-BUF mmap offset must be 0; PipeWire chunk offset applied manually
        )
    }
    .map_err(DmaBufReadError::MmapFailed)?;

    // Begin DMA-BUF read sync
    let sync_start_flags: u64 = DMA_BUF_SYNC_START | DMA_BUF_SYNC_READ;
    if let Err(e) = unsafe { dma_buf_ioctl_sync(fd, &sync_start_flags) } {
        let _ = unsafe { munmap(mapped_ptr, map_size) };
        return Err(DmaBufReadError::SyncFailed(e));
    }

    // Copy pixel data, handling stride and format conversion
    let src_base = mapped_ptr.as_ptr() as *const u8;
    let mut rgba = vec![0u8; row_bytes * height as usize];

    for y in 0..height as usize {
        let src_row = unsafe {
            std::slice::from_raw_parts(
                src_base.add(offset as usize + y * stride as usize),
                row_bytes,
            )
        };
        let dst_row = &mut rgba[y * row_bytes..(y + 1) * row_bytes];

        if needs_bgra_swap {
            // BGRA -> RGBA: swap channels 0 and 2
            for px in 0..width as usize {
                let si = px * 4;
                dst_row[si] = src_row[si + 2]; // R <- B position
                dst_row[si + 1] = src_row[si + 1]; // G
                dst_row[si + 2] = src_row[si]; // B <- R position
                dst_row[si + 3] = if has_padding_alpha {
                    255
                } else {
                    src_row[si + 3]
                };
            }
        } else {
            dst_row.copy_from_slice(src_row);
            if has_padding_alpha {
                for px in 0..width as usize {
                    dst_row[px * 4 + 3] = 255;
                }
            }
        }
    }

    // End DMA-BUF read sync (best-effort)
    let sync_end_flags: u64 = DMA_BUF_SYNC_END | DMA_BUF_SYNC_READ;
    let _ = unsafe { dma_buf_ioctl_sync(fd, &sync_end_flags) };

    // Unmap
    unsafe { munmap(mapped_ptr, map_size) }.map_err(DmaBufReadError::MunmapFailed)?;

    Ok(rgba)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_dmabuf_error_display() {
        use super::DmaBufError;
        let err = DmaBufError::NotAvailable("test".to_string());
        assert!(err.to_string().contains("test"));
    }
}
