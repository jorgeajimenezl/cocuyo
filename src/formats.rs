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
