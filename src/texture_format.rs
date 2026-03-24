/// Map a format to its non-sRGB equivalent (identity for non-sRGB formats).
pub fn non_srgb_equivalent(format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    match format {
        wgpu::TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8Unorm,
        other => other,
    }
}

/// Map a format to its sRGB equivalent (identity for already-sRGB formats).
pub fn srgb_equivalent(format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8UnormSrgb,
        other => other,
    }
}

/// Adjust a format's sRGB-ness to match the render target.
pub fn adjust_srgb(format: wgpu::TextureFormat, target_is_srgb: bool) -> wgpu::TextureFormat {
    if target_is_srgb {
        srgb_equivalent(format)
    } else {
        non_srgb_equivalent(format)
    }
}
