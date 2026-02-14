use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};

use ash::vk;
use drm_fourcc::DrmFourcc;
use tracing::debug;

/// Global flag: once DMA-BUF import fails, stop trying for all subsequent frames.
static DMABUF_IMPORT_FAILED: AtomicBool = AtomicBool::new(false);

/// Returns whether DMA-BUF Vulkan import is still considered viable.
/// Becomes false after the first import failure.
pub fn is_dmabuf_import_available() -> bool {
    !DMABUF_IMPORT_FAILED.load(Ordering::Relaxed)
}

/// Mark DMA-BUF import as failed, so the stream falls back to GStreamer.
pub fn mark_dmabuf_import_failed() {
    DMABUF_IMPORT_FAILED.store(true, Ordering::Relaxed);
}

#[derive(Debug)]
pub enum DmaBufImportError {
    UnsupportedFormat(DrmFourcc),
    VulkanNotAvailable,
    ExtensionNotAvailable(&'static str),
    VulkanError(&'static str, vk::Result),
    NoCompatibleMemoryType,
    FdDupFailed(nix::errno::Errno),
}

impl std::fmt::Display for DmaBufImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(fmt) => write!(f, "Unsupported DRM format: {:?}", fmt),
            Self::VulkanNotAvailable => write!(f, "Vulkan backend not available"),
            Self::ExtensionNotAvailable(ext) => {
                write!(f, "Vulkan extension not available: {}", ext)
            }
            Self::VulkanError(step, e) => write!(f, "Vulkan error at {}: {}", step, e),
            Self::NoCompatibleMemoryType => {
                write!(f, "No compatible memory type for DMA-BUF import")
            }
            Self::FdDupFailed(e) => write!(f, "Failed to dup DMA-BUF fd: {}", e),
        }
    }
}

impl std::error::Error for DmaBufImportError {}

/// Returns whether a DRM format can be directly imported into Vulkan as a 2D texture.
pub fn is_importable_format(format: DrmFourcc) -> bool {
    drm_to_vk_format(format).is_some()
}

/// Maps a DRM fourcc format to the corresponding Vulkan format.
fn drm_to_vk_format(format: DrmFourcc) -> Option<vk::Format> {
    match format {
        DrmFourcc::Xrgb8888 => Some(vk::Format::B8G8R8A8_UNORM), // BGRx
        DrmFourcc::Argb8888 => Some(vk::Format::B8G8R8A8_UNORM), // BGRa
        DrmFourcc::Abgr8888 => Some(vk::Format::R8G8B8A8_UNORM), // RGBA
        DrmFourcc::Xbgr8888 => Some(vk::Format::R8G8B8A8_UNORM), // RGBx
        _ => None,
    }
}

/// Maps a DRM fourcc format to the corresponding wgpu TextureFormat.
pub fn drm_to_wgpu_format(format: DrmFourcc) -> Option<wgpu::TextureFormat> {
    match format {
        DrmFourcc::Xrgb8888 | DrmFourcc::Argb8888 => Some(wgpu::TextureFormat::Bgra8Unorm),
        DrmFourcc::Abgr8888 | DrmFourcc::Xbgr8888 => Some(wgpu::TextureFormat::Rgba8Unorm),
        _ => None,
    }
}

/// Imports a DMA-BUF file descriptor as a wgpu texture via Vulkan external memory.
///
/// The fd is dup'd internally before import — the caller retains ownership of the
/// original fd regardless of success or failure.
///
/// # Safety
///
/// - `fd` must be a valid DMA-BUF file descriptor.
/// - The DMA-BUF must have linear tiling.
/// - The device must be using the Vulkan backend with external memory extensions enabled.
pub unsafe fn import_dmabuf_texture(
    device: &wgpu::Device,
    fd: RawFd,
    width: u32,
    height: u32,
    drm_format: DrmFourcc,
    _stride: u32,
) -> Result<(wgpu::Texture, wgpu::TextureFormat), DmaBufImportError> {
    let vk_format =
        drm_to_vk_format(drm_format).ok_or(DmaBufImportError::UnsupportedFormat(drm_format))?;
    let wgpu_format =
        drm_to_wgpu_format(drm_format).ok_or(DmaBufImportError::UnsupportedFormat(drm_format))?;

    // Dup the fd so Vulkan can take ownership of the copy without affecting the caller's fd.
    // vkAllocateMemory with VkImportMemoryFdInfoKHR transfers fd ownership to Vulkan.
    let import_fd = nix::unistd::dup(fd).map_err(DmaBufImportError::FdDupFailed)?;

    // Access the Vulkan HAL device to perform raw Vulkan operations.
    // We create the hal texture inside this block so we can drop the HAL guard
    // before calling create_texture_from_hal (which also needs the device lock).
    let hal_texture = {
        let hal_guard = match unsafe { device.as_hal::<wgpu_hal::api::Vulkan>() } {
            Some(guard) => guard,
            None => {
                // Close the dup'd fd on failure
                let _ = nix::unistd::close(import_fd);
                return Err(DmaBufImportError::VulkanNotAvailable);
            }
        };

        let ash_device = hal_guard.raw_device();
        let physical_device = hal_guard.raw_physical_device();
        let ash_instance = hal_guard.shared_instance().raw_instance();

        // Check that the required extensions are enabled
        let extensions = hal_guard.enabled_device_extensions();
        let has_external_memory_fd = extensions
            .iter()
            .any(|e| *e == ash::khr::external_memory_fd::NAME);
        let has_dmabuf = extensions
            .iter()
            .any(|e| *e == ash::ext::external_memory_dma_buf::NAME);

        if !has_external_memory_fd {
            let _ = nix::unistd::close(import_fd);
            return Err(DmaBufImportError::ExtensionNotAvailable(
                "VK_KHR_external_memory_fd",
            ));
        }
        if !has_dmabuf {
            let _ = nix::unistd::close(import_fd);
            return Err(DmaBufImportError::ExtensionNotAvailable(
                "VK_EXT_external_memory_dma_buf",
            ));
        }

        // Load extension functions for get_memory_fd_properties
        let ext_memory_fd_fn =
            ash::khr::external_memory_fd::Device::new(ash_instance, ash_device);

        // Query memory properties for this DMA-BUF fd
        let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
        unsafe {
            ext_memory_fd_fn
                .get_memory_fd_properties(
                    vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                    import_fd,
                    &mut fd_properties,
                )
                .map_err(|e| {
                    let _ = nix::unistd::close(import_fd);
                    DmaBufImportError::VulkanError("get_memory_fd_properties", e)
                })?;
        }

        debug!(
            memory_type_bits = fd_properties.memory_type_bits,
            fd = import_fd,
            "DMA-BUF fd memory properties"
        );

        // Create VkImage with external memory support
        let mut external_memory_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

        let image_info = vk::ImageCreateInfo::default()
            .push_next(&mut external_memory_info)
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk_format)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::LINEAR)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::PREINITIALIZED);

        let vk_image = unsafe {
            ash_device.create_image(&image_info, None).map_err(|e| {
                let _ = nix::unistd::close(import_fd);
                DmaBufImportError::VulkanError("create_image", e)
            })?
        };

        // Get memory requirements for the image
        let mem_reqs = unsafe { ash_device.get_image_memory_requirements(vk_image) };

        // Find a compatible memory type
        let mem_properties =
            unsafe { ash_instance.get_physical_device_memory_properties(physical_device) };

        let compatible_bits = mem_reqs.memory_type_bits & fd_properties.memory_type_bits;
        let memory_type_index = find_memory_type_index(
            &mem_properties,
            compatible_bits,
            vk::MemoryPropertyFlags::empty(),
        )
        .ok_or_else(|| {
            unsafe { ash_device.destroy_image(vk_image, None) };
            let _ = nix::unistd::close(import_fd);
            DmaBufImportError::NoCompatibleMemoryType
        })?;

        // Get the actual DMA-BUF size via lseek. When importing external memory,
        // the allocation size must match the actual buffer size, not mem_reqs.size.
        let dmabuf_size = nix::unistd::lseek(import_fd, 0, nix::unistd::Whence::SeekEnd)
            .map(|s| s as u64)
            .unwrap_or(0);
        // Reset seek position
        let _ = nix::unistd::lseek(import_fd, 0, nix::unistd::Whence::SeekSet);

        // Use the actual DMA-BUF size if available, otherwise fall back to mem_reqs.size.
        // The allocation size must be >= mem_reqs.size for the image to be usable.
        let allocation_size = if dmabuf_size > 0 {
            dmabuf_size.max(mem_reqs.size)
        } else {
            mem_reqs.size
        };

        debug!(
            memory_type_index,
            mem_reqs_size = mem_reqs.size,
            dmabuf_size,
            allocation_size,
            "Importing DMA-BUF memory"
        );

        // Import the DMA-BUF fd as Vulkan memory.
        // NOTE: vkAllocateMemory takes ownership of import_fd on SUCCESS.
        // On failure, we must close import_fd ourselves.
        let mut import_info = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(import_fd);

        let alloc_info = vk::MemoryAllocateInfo::default()
            .push_next(&mut import_info)
            .allocation_size(allocation_size)
            .memory_type_index(memory_type_index as u32);

        let memory = unsafe {
            ash_device.allocate_memory(&alloc_info, None).map_err(|e| {
                ash_device.destroy_image(vk_image, None);
                // On allocate_memory failure, Vulkan does NOT take ownership of the fd
                let _ = nix::unistd::close(import_fd);
                DmaBufImportError::VulkanError("allocate_memory (DMA-BUF import)", e)
            })?
        };
        // After successful allocate_memory, Vulkan owns import_fd — do not close it.

        // Bind the imported memory to the image
        unsafe {
            ash_device
                .bind_image_memory(vk_image, memory, 0)
                .map_err(|e| {
                    ash_device.free_memory(memory, None);
                    ash_device.destroy_image(vk_image, None);
                    DmaBufImportError::VulkanError("bind_image_memory", e)
                })?;
        }

        // Create a drop callback to clean up Vulkan resources when the wgpu texture is dropped.
        let cleanup_device = ash_device.clone();
        let drop_callback: wgpu_hal::DropCallback = Box::new(move || unsafe {
            cleanup_device.destroy_image(vk_image, None);
            cleanup_device.free_memory(memory, None);
        });

        // Build the HAL texture descriptor
        let hal_desc = wgpu_hal::TextureDescriptor {
            label: Some("dmabuf_imported"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu_format,
            usage: wgpu::TextureUses::RESOURCE,
            memory_flags: wgpu_hal::MemoryFlags::empty(),
            view_formats: vec![],
        };

        // Wrap the VkImage into a wgpu_hal texture
        unsafe { hal_guard.texture_from_raw(vk_image, &hal_desc, Some(drop_callback)) }
    };
    // HAL guard is dropped here, releasing the device lock

    // Promote to a wgpu::Texture
    let wgpu_desc = wgpu::TextureDescriptor {
        label: Some("dmabuf_imported"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu_format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };

    let wgpu_texture = unsafe {
        device.create_texture_from_hal::<wgpu_hal::api::Vulkan>(hal_texture, &wgpu_desc)
    };

    debug!(
        width,
        height,
        format = ?drm_format,
        "DMA-BUF texture imported successfully"
    );

    Ok((wgpu_texture, wgpu_format))
}

fn find_memory_type_index(
    mem_properties: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required_flags: vk::MemoryPropertyFlags,
) -> Option<usize> {
    for i in 0..mem_properties.memory_type_count as usize {
        if (type_bits & (1 << i)) != 0
            && mem_properties.memory_types[i]
                .property_flags
                .contains(required_flags)
        {
            return Some(i);
        }
    }
    None
}
