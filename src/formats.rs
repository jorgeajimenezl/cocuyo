use ash::vk;
use drm_fourcc::DrmFourcc;
use pipewire::spa;

/// Supported video formats for capture and conversion.
#[allow(dead_code)]
pub const SUPPORTED_FORMATS: &[spa::param::video::VideoFormat] = &[
    spa::param::video::VideoFormat::RGB,
    spa::param::video::VideoFormat::RGBA,
    spa::param::video::VideoFormat::RGBx,
    spa::param::video::VideoFormat::BGRx,
    spa::param::video::VideoFormat::YUY2,
    spa::param::video::VideoFormat::I420,
];

/// Converts a PipeWire video format to a GStreamer format string.
pub fn to_gst_format(format: spa::param::video::VideoFormat) -> Option<&'static str> {
    match format {
        spa::param::video::VideoFormat::RGB => Some("RGB"),
        spa::param::video::VideoFormat::RGBA => Some("RGBA"),
        spa::param::video::VideoFormat::RGBx => Some("RGBx"),
        spa::param::video::VideoFormat::BGRx => Some("BGRx"),
        spa::param::video::VideoFormat::YUY2 => Some("YUY2"),
        spa::param::video::VideoFormat::I420 => Some("I420"),
        _ => None,
    }
}

/// Converts a PipeWire video format to a DRM fourcc code.
pub fn to_drm_format(format: spa::param::video::VideoFormat) -> Option<DrmFourcc> {
    match format {
        spa::param::video::VideoFormat::RGB => Some(DrmFourcc::Rgb888),
        spa::param::video::VideoFormat::RGBA => Some(DrmFourcc::Abgr8888),
        spa::param::video::VideoFormat::RGBx => Some(DrmFourcc::Xbgr8888),
        spa::param::video::VideoFormat::BGRx => Some(DrmFourcc::Xrgb8888),
        spa::param::video::VideoFormat::YUY2 => Some(DrmFourcc::Yuyv),
        spa::param::video::VideoFormat::I420 => Some(DrmFourcc::Yuv420),
        _ => None,
    }
}

/// Returns whether a DRM format can be directly imported into Vulkan as a 2D texture.
pub fn is_importable_format(format: DrmFourcc) -> bool {
    drm_to_vk_format(format).is_some()
}

/// Maps a DRM fourcc format to the corresponding Vulkan format.
pub fn drm_to_vk_format(format: DrmFourcc) -> Option<vk::Format> {
    match format {
        DrmFourcc::Xrgb8888 => Some(vk::Format::B8G8R8A8_SRGB), // BGRx
        DrmFourcc::Argb8888 => Some(vk::Format::B8G8R8A8_SRGB), // BGRa
        DrmFourcc::Abgr8888 => Some(vk::Format::R8G8B8A8_SRGB), // RGBA
        DrmFourcc::Xbgr8888 => Some(vk::Format::R8G8B8A8_SRGB), // RGBx
        _ => None,
    }
}

/// Maps a DRM fourcc format to the corresponding wgpu TextureFormat.
pub fn drm_to_wgpu_format(format: DrmFourcc) -> Option<wgpu::TextureFormat> {
    match format {
        DrmFourcc::Xrgb8888 | DrmFourcc::Argb8888 => Some(wgpu::TextureFormat::Bgra8UnormSrgb),
        DrmFourcc::Abgr8888 | DrmFourcc::Xbgr8888 => Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        _ => None,
    }
}
