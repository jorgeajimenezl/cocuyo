//! GPU import abstraction for [`FrameData`].
//!
//! Hides the per-platform `import_*_texture` calls behind a single trait so
//! that both the video shader widget and [`crate::gpu`] sampler can share one
//! code path. The CPU variant is *not* handled here — it has a different shape
//! (cached texture + `queue.write_texture`) and callers should special-case it
//! before invoking [`GpuImport::import_gpu`].

use cocuyo_core::FrameData;

/// Error returned by [`GpuImport::import_gpu`].
pub type ImportError = Box<dyn std::error::Error + Send + Sync>;

/// Import a frame's native GPU resource as a [`wgpu::Texture`].
pub trait GpuImport {
    /// Import this frame as a wgpu texture, returning the native pixel format.
    ///
    /// Returns `Err` for [`FrameData::Cpu`] — CPU frames must be uploaded via
    /// `queue.write_texture` and the caller is expected to handle that variant
    /// separately.
    fn import_gpu(
        &self,
        device: &wgpu::Device,
    ) -> Result<(wgpu::Texture, wgpu::TextureFormat), ImportError>;

    /// Disable the zero-copy import path for this frame's variant after a
    /// failure. No-op for [`FrameData::Cpu`].
    fn mark_import_failed(&self);
}

impl GpuImport for FrameData {
    fn import_gpu(
        &self,
        device: &wgpu::Device,
    ) -> Result<(wgpu::Texture, wgpu::TextureFormat), ImportError> {
        match self {
            #[cfg(target_os = "linux")]
            FrameData::DmaBuf {
                fd,
                width,
                height,
                drm_format,
                stride,
                offset,
                ..
            } => {
                use std::os::fd::AsRawFd;
                unsafe {
                    cocuyo_platform_linux::vulkan_dmabuf::import_dmabuf_texture(
                        device,
                        fd.as_raw_fd(),
                        *width,
                        *height,
                        *drm_format,
                        *stride,
                        *offset,
                    )
                }
                .map_err(|e| Box::new(e) as ImportError)
            }
            #[cfg(target_os = "macos")]
            FrameData::IOSurface {
                surface,
                width,
                height,
            } => {
                // Wrap Metal/ObjC calls in an autoreleasepool so callers don't
                // leak ObjC objects (and to avoid Cocoa run-loop re-entrancy
                // when invoked from inside the winit event handler).
                screencapturekit::metal::autoreleasepool(|| unsafe {
                    cocuyo_platform_macos::metal_import::import_iosurface_texture(
                        device, surface, *width, *height,
                    )
                })
                .map_err(|e| Box::new(e) as ImportError)
            }
            #[cfg(target_os = "windows")]
            FrameData::D3DShared {
                frame,
                width,
                height,
            } => {
                use windows::Win32::Foundation::HANDLE;
                let handle = HANDLE(frame.shared_handle().0 as *mut core::ffi::c_void);
                unsafe {
                    cocuyo_platform_windows::dx12_import::import_shared_texture(
                        device, handle, *width, *height,
                    )
                }
                .map_err(|e| Box::new(e) as ImportError)
            }
            FrameData::Cpu { .. } => {
                Err("FrameData::Cpu cannot be imported via GpuImport; \
                     upload it directly with queue.write_texture"
                    .into())
            }
        }
    }

    fn mark_import_failed(&self) {
        match self {
            #[cfg(target_os = "linux")]
            FrameData::DmaBuf { .. } => {
                cocuyo_platform_linux::vulkan_dmabuf::mark_dmabuf_import_failed();
            }
            #[cfg(target_os = "macos")]
            FrameData::IOSurface { .. } => {
                cocuyo_platform_macos::metal_import::mark_iosurface_import_failed();
            }
            #[cfg(target_os = "windows")]
            FrameData::D3DShared { .. } => {
                cocuyo_platform_windows::dx12_import::mark_d3d_shared_import_failed();
            }
            FrameData::Cpu { .. } => {}
        }
    }
}
