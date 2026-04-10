//! [`IOSurfaceFrame`] — the macOS zero-copy GPU frame type.
//!
//! Wraps an IOSurface handle produced by ScreenCaptureKit. Implements
//! [`GpuFrame`] so the binary and sampling crate can import it as a Metal
//! texture without knowing macOS specifics.

use screencapturekit::cm::IOSurface;

use cocuyo_core::frame::{GpuFrame, ImportError};

/// An IOSurface-backed frame produced by ScreenCaptureKit.
pub struct IOSurfaceFrame {
    pub surface: IOSurface,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for IOSurfaceFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IOSurfaceFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl GpuFrame for IOSurfaceFrame {
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
        // Wrap Metal/ObjC calls in an autoreleasepool so callers don't leak
        // ObjC objects, and to avoid Cocoa run-loop re-entrancy when invoked
        // from inside the winit event handler.
        screencapturekit::metal::autoreleasepool(|| unsafe {
            crate::metal_import::import_iosurface_texture(
                device,
                &self.surface,
                self.width,
                self.height,
            )
        })
        .map_err(|e| {
            crate::metal_import::mark_iosurface_import_failed();
            ImportError::wrap(e)
        })
    }

    fn read_pixels_bgra(&self) -> Option<Vec<u8>> {
        let guard = self
            .surface
            .lock_read_only()
            .map_err(|e| tracing::error!(error = %e, "Failed to lock IOSurface for CPU readback"))
            .ok()?;
        let bpr = self.surface.bytes_per_row();
        Some(strip_stride_padding(
            guard.as_slice(),
            self.width as usize,
            self.height as usize,
            bpr,
        ))
    }
}

/// Copy BGRA pixel data, stripping row padding if present (moved from `cocuyo-core::macos`).
pub fn strip_stride_padding(
    src: &[u8],
    width: usize,
    height: usize,
    bytes_per_row: usize,
) -> Vec<u8> {
    let stride = width * 4;
    if bytes_per_row == stride {
        let total = stride * height;
        return src[..total.min(src.len())].to_vec();
    }
    let mut bgra = vec![0u8; stride * height];
    for row in 0..height {
        let src_start = row * bytes_per_row;
        if src_start >= src.len() {
            break;
        }
        let available = (src.len() - src_start).min(stride);
        bgra[row * stride..row * stride + available]
            .copy_from_slice(&src[src_start..src_start + available]);
    }
    bgra
}
