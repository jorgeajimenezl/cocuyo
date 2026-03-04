use std::num::NonZeroU64;
use std::sync::Arc;

use tracing::info;

use crate::frame::FrameData;

/// Uniform buffer layout matching the WGSL `Params` struct.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

/// Storage buffer layout for readback (non-atomic on CPU side).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuResult {
    r_sum: u32,
    g_sum: u32,
    b_sum: u32,
    count: u32,
}

struct CachedTexture {
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

/// Result of importing a frame: a texture view plus an optional pending copy
/// that must be recorded into the command encoder before compute dispatches.
struct ImportedFrame {
    view: wgpu::TextureView,
    pending_copy: Option<PendingCopy>,
}

struct PendingCopy {
    src: wgpu::Texture,
    width: u32,
    height: u32,
}

pub struct GpuSampler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    average_pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    result_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    params_stride: usize,
    result_stride: usize,
    buffer_capacity: usize,
    cached_texture: Option<CachedTexture>,
}

#[derive(Debug)]
pub enum GpuSamplerError {
    ImportFailed(String),
    MapFailed,
}

impl std::fmt::Display for GpuSamplerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ImportFailed(msg) => write!(f, "frame import failed: {msg}"),
            Self::MapFailed => write!(f, "buffer mapping failed"),
        }
    }
}

/// Round `size` up to the next multiple of `alignment`.
fn aligned_stride(size: usize, alignment: u32) -> usize {
    let align = alignment as usize;
    (size + align - 1) / align * align
}

impl GpuSampler {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_average_compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_average.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu_sampler_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu_sampler_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let average_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu_average_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let limits = device.limits();
        let params_stride = aligned_stride(
            std::mem::size_of::<Params>(),
            limits.min_uniform_buffer_offset_alignment,
        );
        let result_stride = aligned_stride(
            std::mem::size_of::<GpuResult>(),
            limits.min_storage_buffer_offset_alignment,
        );
        let buffer_capacity = 1;

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_params"),
            size: (params_stride * buffer_capacity) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let result_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_result"),
            size: (result_stride * buffer_capacity) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_readback"),
            size: (result_stride * buffer_capacity) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        info!("GPU sampler created");

        Self {
            device,
            queue,
            average_pipeline,
            bind_group_layout,
            params_buffer,
            result_buffer,
            readback_buffer,
            params_stride,
            result_stride,
            buffer_capacity,
            cached_texture: None,
        }
    }

    /// Ensure buffers are large enough for `count` regions.
    fn ensure_buffers(&mut self, count: usize) {
        if count <= self.buffer_capacity {
            return;
        }

        self.params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_params"),
            size: (self.params_stride * count) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.result_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_result"),
            size: (self.result_stride * count) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_readback"),
            size: (self.result_stride * count) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.buffer_capacity = count;
    }

    /// Sample multiple regions from a frame. Returns one result per region.
    ///
    /// Regions with GPU-capable strategies (Average) are batched into a single
    /// GPU submission. Regions with CPU-only strategies fall back to
    /// `convert_to_cpu()` + CPU sampling.
    pub fn sample_regions(
        &mut self,
        frame: &Arc<FrameData>,
        regions: &[&crate::region::Region],
    ) -> Result<Vec<Option<(u8, u8, u8)>>, GpuSamplerError> {
        let width = frame.width();
        let height = frame.height();

        let mut results: Vec<Option<(u8, u8, u8)>> = vec![None; regions.len()];
        let mut gpu_indices: Vec<usize> = Vec::new();
        let mut cpu_indices: Vec<usize> = Vec::new();

        for (i, region) in regions.iter().enumerate() {
            if region.strategy.supports_gpu() {
                gpu_indices.push(i);
            } else {
                cpu_indices.push(i);
            }
        }

        // GPU path: batch all GPU-capable regions into a single submission
        if !gpu_indices.is_empty() {
            let gpu_count = gpu_indices.len();
            self.ensure_buffers(gpu_count);

            let imported = self.import_frame(frame)?;

            // Build region params and collect valid regions
            let mut valid_slots: Vec<(usize, usize)> = Vec::new(); // (slot, region_idx)
            let mut padded_params = vec![0u8; self.params_stride * gpu_count];

            for (slot, &region_idx) in gpu_indices.iter().enumerate() {
                let region = &regions[region_idx];
                let x0 = (region.x as u32).min(width);
                let y0 = (region.y as u32).min(height);
                let x1 = ((region.x + region.width) as u32).min(width);
                let y1 = ((region.y + region.height) as u32).min(height);

                if x0 >= x1 || y0 >= y1 {
                    continue;
                }

                let params = Params { x0, y0, x1, y1 };
                let offset = slot * self.params_stride;
                padded_params[offset..offset + std::mem::size_of::<Params>()]
                    .copy_from_slice(bytemuck::bytes_of(&params));
                valid_slots.push((slot, region_idx));
            }

            if !valid_slots.is_empty() {
                self.queue.write_buffer(
                    &self.params_buffer,
                    0,
                    &padded_params[..self.params_stride * gpu_count],
                );

                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("gpu_sampler_batch"),
                        });

                // Record pending texture copy into this encoder
                if let Some(copy) = &imported.pending_copy {
                    let ct = self.cached_texture.as_ref().unwrap();
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &copy.src,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &ct.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width: copy.width,
                            height: copy.height,
                            depth_or_array_layers: 1,
                        },
                    );
                }

                // Clear all result slots
                let result_total = (self.result_stride * gpu_count) as u64;
                encoder.clear_buffer(&self.result_buffer, 0, Some(result_total));

                // Record one compute pass per region (all in the same encoder)
                let params_elem_size =
                    NonZeroU64::new(std::mem::size_of::<Params>() as u64).unwrap();
                let result_elem_size =
                    NonZeroU64::new(std::mem::size_of::<GpuResult>() as u64).unwrap();

                for &(slot, region_idx) in &valid_slots {
                    let region = &regions[region_idx];
                    let x0 = (region.x as u32).min(width);
                    let y0 = (region.y as u32).min(height);
                    let x1 = ((region.x + region.width) as u32).min(width);
                    let y1 = ((region.y + region.height) as u32).min(height);

                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("gpu_sampler_bg"),
                        layout: &self.bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&imported.view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &self.params_buffer,
                                    offset: (slot * self.params_stride) as u64,
                                    size: Some(params_elem_size),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &self.result_buffer,
                                    offset: (slot * self.result_stride) as u64,
                                    size: Some(result_elem_size),
                                }),
                            },
                        ],
                    });

                    let workgroups_x = (x1 - x0).div_ceil(16);
                    let workgroups_y = (y1 - y0).div_ceil(16);

                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("gpu_average_pass"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(&self.average_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
                    }
                }

                // Single copy from result to readback
                encoder.copy_buffer_to_buffer(
                    &self.result_buffer,
                    0,
                    &self.readback_buffer,
                    0,
                    result_total,
                );

                // Single submit
                self.queue.submit(std::iter::once(encoder.finish()));

                // Single map + poll
                let readback_slice = self.readback_buffer.slice(..result_total);
                let (sender, mut receiver) = futures::channel::oneshot::channel();
                readback_slice.map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
                let _ = self.device.poll(wgpu::PollType::wait_indefinitely());

                receiver
                    .try_recv()
                    .ok()
                    .flatten()
                    .and_then(|r| r.ok())
                    .ok_or(GpuSamplerError::MapFailed)?;

                // Read all results at stride offsets
                let mapped = readback_slice.get_mapped_range();
                for &(slot, region_idx) in &valid_slots {
                    let offset = slot * self.result_stride;
                    let result: GpuResult = *bytemuck::from_bytes(
                        &mapped[offset..offset + std::mem::size_of::<GpuResult>()],
                    );
                    if result.count > 0 {
                        results[region_idx] = Some((
                            (result.r_sum / result.count) as u8,
                            (result.g_sum / result.count) as u8,
                            (result.b_sum / result.count) as u8,
                        ));
                    }
                }
                drop(mapped);
                self.readback_buffer.unmap();
            }
        }

        // CPU fallback for unsupported strategies
        if !cpu_indices.is_empty() {
            if let Some(cpu_frame) = frame.convert_to_cpu() {
                for &i in &cpu_indices {
                    let region = &regions[i];
                    results[i] = super::sample_region(
                        &cpu_frame,
                        region.x,
                        region.y,
                        region.width,
                        region.height,
                        &region.strategy,
                    );
                }
            }
        }

        Ok(results)
    }

    /// Import a frame as a GPU texture, returning a view and optional pending
    /// texture copy to be recorded into the caller's command encoder.
    fn import_frame(&mut self, frame: &Arc<FrameData>) -> Result<ImportedFrame, GpuSamplerError> {
        match frame.as_ref() {
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
                let (imported, wgpu_format) = unsafe {
                    crate::platform::linux::vulkan_dmabuf::import_dmabuf_texture(
                        &self.device,
                        fd.as_raw_fd(),
                        *width,
                        *height,
                        *drm_format,
                        *stride,
                        *offset,
                    )
                }
                .map_err(|e| GpuSamplerError::ImportFailed(e.to_string()))?;

                self.ensure_texture(*width, *height, wgpu_format);
                let view = create_non_srgb_view(
                    &self.cached_texture.as_ref().unwrap().texture,
                    wgpu_format,
                );
                Ok(ImportedFrame {
                    view,
                    pending_copy: Some(PendingCopy {
                        src: imported,
                        width: *width,
                        height: *height,
                    }),
                })
            }
            #[cfg(target_os = "windows")]
            FrameData::D3DShared {
                slot,
                width,
                height,
            } => {
                use windows::Win32::Foundation::HANDLE;
                let handle = HANDLE(slot.shared_handle.0 as *mut core::ffi::c_void);
                let (imported, wgpu_format) = unsafe {
                    crate::platform::windows::dx12_import::import_shared_texture(
                        &self.device,
                        handle,
                        *width,
                        *height,
                    )
                }
                .map_err(|e| GpuSamplerError::ImportFailed(e.to_string()))?;

                self.ensure_texture(*width, *height, wgpu_format);
                let view = create_non_srgb_view(
                    &self.cached_texture.as_ref().unwrap().texture,
                    wgpu_format,
                );
                Ok(ImportedFrame {
                    view,
                    pending_copy: Some(PendingCopy {
                        src: imported,
                        width: *width,
                        height: *height,
                    }),
                })
            }
            FrameData::Cpu {
                data,
                width,
                height,
            } => {
                let format = wgpu::TextureFormat::Rgba8UnormSrgb;
                self.ensure_texture(*width, *height, format);
                let ct = self.cached_texture.as_ref().unwrap();
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &ct.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(*width * 4),
                        rows_per_image: Some(*height),
                    },
                    wgpu::Extent3d {
                        width: *width,
                        height: *height,
                        depth_or_array_layers: 1,
                    },
                );
                Ok(ImportedFrame {
                    view: create_non_srgb_view(&ct.texture, format),
                    pending_copy: None,
                })
            }
        }
    }

    /// Ensure the cached texture exists with the given dimensions and format.
    fn ensure_texture(&mut self, width: u32, height: u32, format: wgpu::TextureFormat) {
        let needs_recreate = match &self.cached_texture {
            Some(ct) => ct.width != width || ct.height != height || ct.format != format,
            None => true,
        };

        if needs_recreate {
            let non_srgb = non_srgb_equivalent(format);
            let mut view_formats = vec![];
            if non_srgb != format {
                view_formats.push(non_srgb);
            }

            self.cached_texture = Some(CachedTexture {
                texture: self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("gpu_sampler_frame"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &view_formats,
                }),
                width,
                height,
                format,
            });
        }
    }
}

/// Create a non-sRGB texture view so `textureLoad` returns raw byte values.
fn create_non_srgb_view(
    texture: &wgpu::Texture,
    original_format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    let view_format = non_srgb_equivalent(original_format);
    texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(view_format),
        ..Default::default()
    })
}

/// Map an sRGB format to its non-sRGB equivalent so `textureLoad` returns raw values.
fn non_srgb_equivalent(format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    match format {
        wgpu::TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8Unorm,
        other => other,
    }
}
