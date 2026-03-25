use tracing::debug;
use windows::Win32::Foundation::HANDLE;

use crate::frame::ImportGuard;

static IMPORT_GUARD: ImportGuard = ImportGuard::new();

pub fn is_d3d_shared_import_available() -> bool {
    IMPORT_GUARD.is_available()
}

pub fn mark_d3d_shared_import_failed() {
    IMPORT_GUARD.mark_failed();
}

pub fn reset_d3d_shared_import_failed() {
    IMPORT_GUARD.reset();
}

#[derive(Debug)]
pub enum Dx12ImportError {
    Dx12HalNotAvailable,
    OpenSharedHandleFailed(windows::core::Error),
}

impl std::fmt::Display for Dx12ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dx12HalNotAvailable => write!(f, "DX12 HAL backend not available"),
            Self::OpenSharedHandleFailed(e) => write!(f, "OpenSharedHandle failed: {e}"),
        }
    }
}

impl std::error::Error for Dx12ImportError {}

/// Import a shared D3D11 texture handle as a `wgpu::Texture` via the DX12 HAL.
///
/// The shared handle must be an NT handle from `IDXGIResource1::CreateSharedHandle`
/// on the same adapter that wgpu is using.
///
/// # Safety
///
/// - `shared_handle` must be a valid NT shared handle.
/// - The wgpu device must be using the DX12 backend.
pub unsafe fn import_shared_texture(
    device: &wgpu::Device,
    shared_handle: HANDLE,
    width: u32,
    height: u32,
) -> Result<(wgpu::Texture, wgpu::TextureFormat), Dx12ImportError> {
    let wgpu_format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };

    // Access the DX12 HAL device.  The guard holds a device lock that we
    // must release before calling create_texture_from_hal.
    let hal_texture = {
        let hal_guard = match unsafe { device.as_hal::<wgpu_hal::api::Dx12>() } {
            Some(guard) => guard,
            None => return Err(Dx12ImportError::Dx12HalNotAvailable),
        };

        let dx12_device = hal_guard.raw_device();

        // Open the NT shared handle as an ID3D12Resource.
        let resource = {
            let mut resource: Option<windows::Win32::Graphics::Direct3D12::ID3D12Resource> = None;
            debug!(
                ?shared_handle,
                width, height, "Opening shared handle via DX12"
            );
            unsafe { dx12_device.OpenSharedHandle(shared_handle, &mut resource as *mut _) }
                .map_err(Dx12ImportError::OpenSharedHandleFailed)?;
            resource.ok_or(Dx12ImportError::OpenSharedHandleFailed(
                windows::core::Error::from_hresult(windows::core::HRESULT(-1)),
            ))?
        };

        // Wrap as a wgpu_hal texture.  texture_from_raw takes ownership of
        // the COM reference; cleanup happens when the hal texture is dropped.
        unsafe {
            wgpu_hal::dx12::Device::texture_from_raw(
                resource,
                wgpu_format,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            )
        }
    };
    // HAL guard dropped — device lock released.

    let non_srgb = crate::texture_format::non_srgb_equivalent(wgpu_format);
    let alt_view_arr = (non_srgb != wgpu_format).then_some([non_srgb]);
    let alt_view_slice: &[wgpu::TextureFormat] =
        alt_view_arr.as_ref().map_or(&[], |a| a.as_slice());

    let wgpu_desc = wgpu::TextureDescriptor {
        label: Some("d3d_shared_imported"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu_format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: alt_view_slice,
    };

    let wgpu_texture =
        unsafe { device.create_texture_from_hal::<wgpu_hal::api::Dx12>(hal_texture, &wgpu_desc) };

    debug!(width, height, "D3D shared texture imported via DX12");

    Ok((wgpu_texture, wgpu_format))
}
