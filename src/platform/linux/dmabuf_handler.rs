use nix::sys::stat::fstat;
use pipewire as pw;
use std::os::fd::RawFd;

use super::formats::to_drm_format;

#[derive(Debug, Clone)]
pub struct DmaBufBuffer {
    pub fd: RawFd,
    pub width: u32,
    pub height: u32,
    pub format: drm_fourcc::DrmFourcc,
    pub stride: u32,
    #[allow(dead_code)]
    pub offset: u32,
    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_dmabuf_error_display() {
        use super::DmaBufError;
        let err = DmaBufError::NotAvailable("test".to_string());
        assert!(err.to_string().contains("test"));
    }
}
