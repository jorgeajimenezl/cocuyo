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

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat;

    #[test]
    fn non_srgb_equivalent_maps_srgb_formats() {
        assert_eq!(
            non_srgb_equivalent(TextureFormat::Bgra8UnormSrgb),
            TextureFormat::Bgra8Unorm
        );
        assert_eq!(
            non_srgb_equivalent(TextureFormat::Rgba8UnormSrgb),
            TextureFormat::Rgba8Unorm
        );
    }

    #[test]
    fn non_srgb_equivalent_identity_for_non_srgb() {
        assert_eq!(
            non_srgb_equivalent(TextureFormat::Bgra8Unorm),
            TextureFormat::Bgra8Unorm
        );
        assert_eq!(
            non_srgb_equivalent(TextureFormat::Rgba8Unorm),
            TextureFormat::Rgba8Unorm
        );
    }

    #[test]
    fn srgb_equivalent_maps_non_srgb_formats() {
        assert_eq!(
            srgb_equivalent(TextureFormat::Bgra8Unorm),
            TextureFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            srgb_equivalent(TextureFormat::Rgba8Unorm),
            TextureFormat::Rgba8UnormSrgb
        );
    }

    #[test]
    fn srgb_equivalent_identity_for_srgb() {
        assert_eq!(
            srgb_equivalent(TextureFormat::Bgra8UnormSrgb),
            TextureFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            srgb_equivalent(TextureFormat::Rgba8UnormSrgb),
            TextureFormat::Rgba8UnormSrgb
        );
    }

    #[test]
    fn adjust_srgb_selects_correct_variant() {
        assert_eq!(
            adjust_srgb(TextureFormat::Bgra8UnormSrgb, false),
            TextureFormat::Bgra8Unorm
        );
        assert_eq!(
            adjust_srgb(TextureFormat::Bgra8Unorm, true),
            TextureFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            adjust_srgb(TextureFormat::Rgba8Unorm, false),
            TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            adjust_srgb(TextureFormat::Rgba8UnormSrgb, true),
            TextureFormat::Rgba8UnormSrgb
        );
    }
}
