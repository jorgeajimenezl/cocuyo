//! [`HeldFrame`] — the Windows zero-copy GPU frame type (moved from `cocuyo-core::windows`).
//!
//! Holds a WGC captured texture alive via NT shared handle so the binary and
//! sampling crate can import it into wgpu through [`GpuFrame`] without knowing
//! any D3D details.

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows_capture::HeldCaptureFrame;

use cocuyo_core::frame::{GpuFrame, ImportError};

/// A captured frame ready for DX12 import via NT shared handle.
///
/// Holds the WGC capture frame alive so the texture remains valid,
/// plus the shared NT handle for cross-API import.
pub struct HeldFrame {
    _held: HeldCaptureFrame,
    texture: ID3D11Texture2D,
    shared_handle: HANDLE,
    width: u32,
    height: u32,
}

unsafe impl Send for HeldFrame {}
unsafe impl Sync for HeldFrame {}

impl std::fmt::Debug for HeldFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl HeldFrame {
    pub fn new(
        held: HeldCaptureFrame,
        texture: ID3D11Texture2D,
        shared_handle: HANDLE,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            _held: held,
            texture,
            shared_handle,
            width,
            height,
        }
    }

    pub fn shared_handle(&self) -> HANDLE {
        self.shared_handle
    }

    pub fn read_pixels(&self) -> Result<Vec<u8>, SharedTextureError> {
        read_texture_pixels(&self.texture, self.width, self.height)
    }
}

impl Drop for HeldFrame {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.shared_handle);
        }
    }
}

impl GpuFrame for HeldFrame {
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
            crate::dx12_import::import_shared_texture(
                device,
                self.shared_handle,
                self.width,
                self.height,
            )
        }
        .map_err(|e| {
            crate::dx12_import::mark_d3d_shared_import_failed();
            ImportError::wrap(e)
        })
    }

    fn read_pixels_bgra(&self) -> Option<Vec<u8>> {
        self.read_pixels()
            .map_err(|e| tracing::error!(error = %e, "shared texture pixel readback failed"))
            .ok()
    }
}

fn read_texture_pixels(
    texture: &ID3D11Texture2D,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, SharedTextureError> {
    let device = unsafe { texture.GetDevice()? };
    let context = unsafe { device.GetImmediateContext()? };

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };

    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe {
        device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
    }
    let staging = staging.ok_or_else(|| {
        SharedTextureError::Windows(windows::core::Error::from_hresult(windows::core::HRESULT(
            -1,
        )))
    })?;

    unsafe {
        context.CopyResource(&staging, texture);
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    }

    let row_pitch = mapped.RowPitch as usize;
    let row_bytes = (width as usize) * 4;
    let mut bgra = vec![0u8; row_bytes * height as usize];

    for y in 0..height as usize {
        let src = unsafe {
            std::slice::from_raw_parts((mapped.pData as *const u8).add(y * row_pitch), row_bytes)
        };
        bgra[y * row_bytes..(y + 1) * row_bytes].copy_from_slice(src);
    }

    unsafe {
        context.Unmap(&staging, 0);
    }

    Ok(bgra)
}

#[derive(Debug)]
pub enum SharedTextureError {
    Windows(windows::core::Error),
}

impl std::fmt::Display for SharedTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Windows(e) => write!(f, "Windows error: {e}"),
        }
    }
}

impl std::error::Error for SharedTextureError {}

impl From<windows::core::Error> for SharedTextureError {
    fn from(e: windows::core::Error) -> Self {
        Self::Windows(e)
    }
}
