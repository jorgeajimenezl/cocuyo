//! DMA-BUF pixel readback helpers (moved from `cocuyo-core::linux`).
//!
//! Used by [`super::dmabuf_frame::DmaBufFrame::read_pixels_bgra`] as the
//! CPU fallback when GPU sampling strategies need raw pixel data.

use std::num::NonZeroUsize;
use std::os::fd::{BorrowedFd, RawFd};

// DMA-BUF sync ioctl constants
const DMA_BUF_SYNC_READ: u64 = 1 << 0;
const DMA_BUF_SYNC_START: u64 = 0 << 2;
const DMA_BUF_SYNC_END: u64 = 1 << 2;

nix::ioctl_write_ptr!(dma_buf_ioctl_sync, b'b', 0, u64);

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

/// Reads pixel data from a DMA-BUF fd via mmap, returning tightly-packed BGRA bytes.
///
/// Handles stride padding (stride may be > width * 4) and normalizes RGBA
/// formats (Abgr8888, Xbgr8888) to BGRA byte order.
pub fn read_dmabuf_pixels(
    fd: RawFd,
    width: u32,
    height: u32,
    stride: u32,
    offset: u32,
    format: drm_fourcc::DrmFourcc,
) -> Result<Vec<u8>, DmaBufReadError> {
    use drm_fourcc::DrmFourcc;
    use nix::sys::mman::{MapFlags, ProtFlags, mmap, munmap};

    let needs_rgba_swap = matches!(format, DrmFourcc::Abgr8888 | DrmFourcc::Xbgr8888);
    let has_padding_alpha = matches!(format, DrmFourcc::Xrgb8888 | DrmFourcc::Xbgr8888);

    if !matches!(
        format,
        DrmFourcc::Xrgb8888 | DrmFourcc::Argb8888 | DrmFourcc::Abgr8888 | DrmFourcc::Xbgr8888
    ) {
        return Err(DmaBufReadError::UnsupportedFormat(format));
    }

    let row_bytes = (width as usize) * 4;
    if (stride as usize) < row_bytes {
        return Err(DmaBufReadError::InvalidBufferSize);
    }
    let map_size = offset as usize + (stride as usize) * (height as usize);

    let map_len = NonZeroUsize::new(map_size).ok_or(DmaBufReadError::InvalidBufferSize)?;

    // SAFETY: fd is a valid DMA-BUF fd during the PipeWire on_process callback.
    let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
    let mapped_ptr = unsafe {
        mmap(
            None,
            map_len,
            ProtFlags::PROT_READ,
            MapFlags::MAP_SHARED,
            borrowed_fd,
            0,
        )
    }
    .map_err(DmaBufReadError::MmapFailed)?;

    let sync_start_flags: u64 = DMA_BUF_SYNC_START | DMA_BUF_SYNC_READ;
    if let Err(e) = unsafe { dma_buf_ioctl_sync(fd, &sync_start_flags) } {
        let _ = unsafe { munmap(mapped_ptr, map_size) };
        return Err(DmaBufReadError::SyncFailed(e));
    }

    let src_base = mapped_ptr.as_ptr() as *const u8;
    let mut bgra = vec![0u8; row_bytes * height as usize];

    for y in 0..height as usize {
        let src_row = unsafe {
            std::slice::from_raw_parts(
                src_base.add(offset as usize + y * stride as usize),
                row_bytes,
            )
        };
        let dst_row = &mut bgra[y * row_bytes..(y + 1) * row_bytes];

        if needs_rgba_swap {
            for px in 0..width as usize {
                let si = px * 4;
                dst_row[si] = src_row[si + 2];
                dst_row[si + 1] = src_row[si + 1];
                dst_row[si + 2] = src_row[si];
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

    let sync_end_flags: u64 = DMA_BUF_SYNC_END | DMA_BUF_SYNC_READ;
    let _ = unsafe { dma_buf_ioctl_sync(fd, &sync_end_flags) };

    unsafe { munmap(mapped_ptr, map_size) }.map_err(DmaBufReadError::MunmapFailed)?;

    Ok(bgra)
}
