use foreign_types::ForeignType;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use screencapturekit::cm::IOSurface;
use tracing::debug;

use crate::frame::ImportGuard;

// Metal API constants
const MTL_PIXEL_FORMAT_BGRA8_UNORM_SRGB: u64 = 81;
const MTL_TEXTURE_USAGE_SHADER_READ_RENDER_TARGET: u64 = 0x01 | 0x04; // ShaderRead | RenderTarget
const MTL_STORAGE_MODE_SHARED: u64 = 1;

static IMPORT_GUARD: ImportGuard = ImportGuard::new();

pub fn is_iosurface_import_available() -> bool {
    IMPORT_GUARD.is_available()
}

pub fn mark_iosurface_import_failed() {
    IMPORT_GUARD.mark_failed()
}

pub fn reset_iosurface_import_failed() {
    IMPORT_GUARD.reset()
}

#[derive(Debug)]
pub enum MetalImportError {
    MetalHalNotAvailable,
    TextureCreationFailed,
}

impl std::fmt::Display for MetalImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MetalHalNotAvailable => write!(f, "Metal HAL backend not available"),
            Self::TextureCreationFailed => {
                write!(f, "Failed to create Metal texture from IOSurface")
            }
        }
    }
}

impl std::error::Error for MetalImportError {}

/// Import an IOSurface as a `wgpu::Texture` via the Metal HAL.
///
/// Uses `[MTLDevice newTextureWithDescriptor:iosurface:plane:]` to create
/// a Metal texture backed by the IOSurface without copying pixel data.
///
/// # Safety
///
/// - The IOSurface must contain valid pixel data.
/// - The wgpu device must be using the Metal backend.
pub unsafe fn import_iosurface_texture(
    device: &wgpu::Device,
    surface: &IOSurface,
    width: u32,
    height: u32,
) -> Result<(wgpu::Texture, wgpu::TextureFormat), MetalImportError> {
    let wgpu_format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };

    // Access the Metal HAL device. The guard holds a device lock that we
    // must release before calling create_texture_from_hal.
    let hal_texture = {
        let hal_guard = match unsafe { device.as_hal::<wgpu_hal::api::Metal>() } {
            Some(guard) => guard,
            None => return Err(MetalImportError::MetalHalNotAvailable),
        };

        let metal_device = hal_guard.raw_device();

        // Get raw pointer to the MTLDevice ObjC object
        let device_ptr = metal_device.as_ptr() as *mut AnyObject;

        let descriptor: *mut AnyObject = unsafe {
            msg_send![class!(MTLTextureDescriptor),
                texture2DDescriptorWithPixelFormat: MTL_PIXEL_FORMAT_BGRA8_UNORM_SRGB,
                width: width as u64,
                height: height as u64,
                mipmapped: false]
        };

        if descriptor.is_null() {
            return Err(MetalImportError::TextureCreationFailed);
        }

        // RenderTarget is needed because wgpu maps COPY_SRC to it on Metal (used by GPU sampler).
        let _: () = unsafe {
            msg_send![&*descriptor, setUsage: MTL_TEXTURE_USAGE_SHADER_READ_RENDER_TARGET]
        };
        // IOSurface-backed textures require shared storage mode.
        let _: () = unsafe { msg_send![&*descriptor, setStorageMode: MTL_STORAGE_MODE_SHARED] };

        // Create Metal texture from IOSurface via
        // [MTLDevice newTextureWithDescriptor:iosurface:plane:]
        let iosurface_ptr = surface.as_ptr();
        let metal_texture_ptr: *mut AnyObject = unsafe {
            msg_send![&*device_ptr, newTextureWithDescriptor: descriptor,
                iosurface: iosurface_ptr,
                plane: 0u64]
        };

        if metal_texture_ptr.is_null() {
            return Err(MetalImportError::TextureCreationFailed);
        }

        // Convert raw pointer to metal::Texture.
        // msg_send returns a retained object (+1); from_ptr takes ownership.
        let metal_texture = unsafe { metal::Texture::from_ptr(metal_texture_ptr as *mut _) };

        // Wrap as a wgpu_hal texture.
        unsafe {
            wgpu_hal::metal::Device::texture_from_raw(
                metal_texture,
                wgpu_format,
                metal::MTLTextureType::D2,
                1, // array_layers
                1, // mip_levels
                wgpu_hal::CopyExtent {
                    width,
                    height,
                    depth: 1,
                },
            )
        }
    };
    // HAL guard dropped — device lock released.

    let non_srgb = crate::texture_format::non_srgb_equivalent(wgpu_format);
    let alt_view_arr = (non_srgb != wgpu_format).then_some([non_srgb]);
    let alt_view_slice: &[wgpu::TextureFormat] =
        alt_view_arr.as_ref().map_or(&[], |a| a.as_slice());

    let wgpu_desc = wgpu::TextureDescriptor {
        label: Some("iosurface_imported"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu_format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: alt_view_slice,
    };

    let wgpu_texture =
        unsafe { device.create_texture_from_hal::<wgpu_hal::api::Metal>(hal_texture, &wgpu_desc) };

    debug!(width, height, "IOSurface imported via Metal");

    Ok((wgpu_texture, wgpu_format))
}
