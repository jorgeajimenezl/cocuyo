use std::num::NonZeroU64;
use std::sync::Arc;

use tracing::info;

use crate::frame::FrameData;

use super::BoxedStrategy;

/// Lightweight region parameters for sending to the background sampling thread.
#[derive(Clone)]
pub struct RegionParams {
    pub region_id: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub supports_gpu: bool,
    pub strategy: BoxedStrategy,
}

/// Result of try_send on SamplingWorker.
pub enum SendResult<M> {
    /// Request accepted; returns a Task that resolves to the sampling result.
    Sent(iced::Task<M>),
    /// Worker is still processing the previous frame.
    Busy,
    /// Worker thread has died (channel disconnected).
    Dead,
}

struct SamplingRequest {
    frame: Arc<FrameData>,
    regions: Vec<RegionParams>,
    result_tx: tokio::sync::oneshot::Sender<Vec<(usize, Option<(u8, u8, u8)>)>>,
}

/// Handle to a background GPU sampling thread.
///
/// Dropping this struct drops the request channel sender, which causes the
/// background thread's `recv()` to return `Err` and exit cleanly.
pub struct SamplingWorker {
    request_tx: std::sync::mpsc::SyncSender<SamplingRequest>,
    _thread: std::thread::JoinHandle<()>,
}

impl SamplingWorker {
    pub fn spawn(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<SamplingRequest>(1);

        let thread = std::thread::Builder::new()
            .name("gpu-sampler".to_string())
            .spawn(move || {
                let mut sampler = GpuSampler::new(device, queue);
                while let Ok(request) = request_rx.recv() {
                    let results = sampler.sample_regions(&request.frame, &request.regions);
                    let colors = match results {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(error = %e, "GPU sampling failed");
                            request
                                .regions
                                .iter()
                                .map(|r| (r.region_id, None))
                                .collect()
                        }
                    };
                    let _ = request.result_tx.send(colors);
                }
                info!("GPU sampler worker thread exiting");
            })
            .expect("failed to spawn gpu-sampler thread");

        Self {
            request_tx,
            _thread: thread,
        }
    }

    /// Try to submit a sampling request.
    ///
    /// Returns `Sent(task)` with an iced `Task` that resolves to the result
    /// message, `Busy` if the worker is still processing, or `Dead` if the
    /// worker thread has exited.
    pub fn try_send<M: Send + 'static>(
        &self,
        frame: Arc<FrameData>,
        regions: Vec<RegionParams>,
        map_fn: fn(Vec<(usize, Option<(u8, u8, u8)>)>) -> M,
    ) -> SendResult<M> {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let request = SamplingRequest {
            frame,
            regions,
            result_tx,
        };
        match self.request_tx.try_send(request) {
            Ok(()) => SendResult::Sent(iced::Task::perform(
                async move { result_rx.await.unwrap_or_default() },
                map_fn,
            )),
            Err(std::sync::mpsc::TrySendError::Full(_)) => SendResult::Busy,
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => SendResult::Dead,
        }
    }
}

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

/// Histogram bin for palette quantization readback (512 bins, non-atomic CPU side).
#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct HistogramBin {
    pub r_sum: u32,
    pub g_sum: u32,
    pub b_sum: u32,
    pub count: u32,
}

pub(super) const PALETTE_BINS: usize = 512;
const PALETTE_RESULT_SIZE: usize = PALETTE_BINS * std::mem::size_of::<HistogramBin>();

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

struct GpuSampler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    average_pipeline: wgpu::ComputePipeline,
    palette_pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    params_stride: usize,
    params_capacity: usize,
    // Average strategy buffers
    avg_result_buffer: wgpu::Buffer,
    avg_readback_buffer: wgpu::Buffer,
    avg_result_stride: usize,
    avg_buffer_capacity: usize,
    // Palette strategy buffers
    palette_result_buffer: wgpu::Buffer,
    palette_readback_buffer: wgpu::Buffer,
    palette_result_stride: usize,
    palette_buffer_capacity: usize,
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
    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let avg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_average_compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_average.wgsl").into()),
        });

        let palette_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_palette_compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_palette.wgsl").into()),
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
            module: &avg_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let palette_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu_palette_pipeline"),
            layout: Some(&pipeline_layout),
            module: &palette_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let limits = device.limits();
        let params_stride = aligned_stride(
            std::mem::size_of::<Params>(),
            limits.min_uniform_buffer_offset_alignment,
        );
        let avg_result_stride = aligned_stride(
            std::mem::size_of::<GpuResult>(),
            limits.min_storage_buffer_offset_alignment,
        );
        let palette_result_stride = aligned_stride(
            PALETTE_RESULT_SIZE,
            limits.min_storage_buffer_offset_alignment,
        );
        let initial_capacity = 1;

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_params"),
            size: (params_stride * initial_capacity) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let avg_result_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_avg_result"),
            size: (avg_result_stride * initial_capacity) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let avg_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_avg_readback"),
            size: (avg_result_stride * initial_capacity) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let palette_result_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_palette_result"),
            size: (palette_result_stride * initial_capacity) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let palette_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_palette_readback"),
            size: (palette_result_stride * initial_capacity) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        info!("GPU sampler created");

        Self {
            device,
            queue,
            average_pipeline,
            palette_pipeline,
            bind_group_layout,
            params_buffer,
            params_stride,
            params_capacity: initial_capacity,
            avg_result_buffer,
            avg_readback_buffer,
            avg_result_stride,
            avg_buffer_capacity: initial_capacity,
            palette_result_buffer,
            palette_readback_buffer,
            palette_result_stride,
            palette_buffer_capacity: initial_capacity,
            cached_texture: None,
        }
    }

    /// Ensure params buffer is large enough for `count` GPU regions (shared across strategies).
    fn ensure_params_buffer(&mut self, count: usize) {
        if count <= self.params_capacity {
            return;
        }
        self.params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_params"),
            size: (self.params_stride * count) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.params_capacity = count;
    }

    /// Ensure average result/readback buffers are large enough for `count` regions.
    fn ensure_avg_buffers(&mut self, count: usize) {
        if count == 0 || count <= self.avg_buffer_capacity {
            return;
        }
        self.avg_result_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_avg_result"),
            size: (self.avg_result_stride * count) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.avg_readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_avg_readback"),
            size: (self.avg_result_stride * count) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.avg_buffer_capacity = count;
    }

    /// Ensure palette result/readback buffers are large enough for `count` regions.
    fn ensure_palette_buffers(&mut self, count: usize) {
        if count == 0 || count <= self.palette_buffer_capacity {
            return;
        }
        self.palette_result_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_palette_result"),
            size: (self.palette_result_stride * count) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.palette_readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_palette_readback"),
            size: (self.palette_result_stride * count) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.palette_buffer_capacity = count;
    }

    /// Sample multiple regions from a frame. Returns `(region_id, color)` pairs.
    ///
    /// GPU-capable regions are batched into a single GPU submission using
    /// per-strategy pipelines (average vs palette). Regions with CPU-only
    /// strategies fall back to `convert_to_cpu()` + CPU sampling.
    fn sample_regions(
        &mut self,
        frame: &Arc<FrameData>,
        regions: &[RegionParams],
    ) -> Result<Vec<(usize, Option<(u8, u8, u8)>)>, GpuSamplerError> {
        let width = frame.width();
        let height = frame.height();

        let mut results: Vec<Option<(u8, u8, u8)>> = vec![None; regions.len()];
        let mut cpu_indices: Vec<usize> = Vec::new();

        // Classify GPU regions by strategy, assigning slots in their respective
        // result buffers. All GPU regions share contiguous params buffer slots.
        // (params_slot, result_slot, region_idx)
        let mut avg_slots: Vec<(usize, usize, usize)> = Vec::new();
        let mut palette_slots: Vec<(usize, usize, usize)> = Vec::new();
        let mut gpu_slot: usize = 0;

        for (i, region) in regions.iter().enumerate() {
            if !region.supports_gpu {
                cpu_indices.push(i);
            } else {
                let ps = gpu_slot;
                gpu_slot += 1;
                if region.strategy.id() == "palette" {
                    palette_slots.push((ps, palette_slots.len(), i));
                } else {
                    avg_slots.push((ps, avg_slots.len(), i));
                }
            }
        }

        let gpu_count = gpu_slot;

        // GPU path: batch all GPU-capable regions into a single submission
        if gpu_count > 0 {
            self.ensure_params_buffer(gpu_count);
            self.ensure_avg_buffers(avg_slots.len());
            self.ensure_palette_buffers(palette_slots.len());

            let imported = self.import_frame(frame)?;

            // Build params for ALL gpu regions (shared buffer, contiguous slots)
            let mut padded_params = vec![0u8; self.params_stride * gpu_count];
            // Track which slots are valid (non-empty region bounds)
            let mut valid_avg: Vec<(usize, usize, usize, Params)> = Vec::new();
            let mut valid_palette: Vec<(usize, usize, usize, Params)> = Vec::new();

            let clamp_region = |rp: &RegionParams| -> Option<Params> {
                let x0 = (rp.x as u32).min(width);
                let y0 = (rp.y as u32).min(height);
                let x1 = ((rp.x + rp.width) as u32).min(width);
                let y1 = ((rp.y + rp.height) as u32).min(height);
                if x0 >= x1 || y0 >= y1 {
                    None
                } else {
                    Some(Params { x0, y0, x1, y1 })
                }
            };

            for &(ps, rs, ri) in &avg_slots {
                if let Some(params) = clamp_region(&regions[ri]) {
                    let offset = ps * self.params_stride;
                    padded_params[offset..offset + std::mem::size_of::<Params>()]
                        .copy_from_slice(bytemuck::bytes_of(&params));
                    valid_avg.push((ps, rs, ri, params));
                }
            }
            for &(ps, rs, ri) in &palette_slots {
                if let Some(params) = clamp_region(&regions[ri]) {
                    let offset = ps * self.params_stride;
                    padded_params[offset..offset + std::mem::size_of::<Params>()]
                        .copy_from_slice(bytemuck::bytes_of(&params));
                    valid_palette.push((ps, rs, ri, params));
                }
            }

            let has_avg = !valid_avg.is_empty();
            let has_palette = !valid_palette.is_empty();

            if has_avg || has_palette {
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

                // Record pending texture copy
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

                // Clear result buffers
                if has_avg {
                    let total = (self.avg_result_stride * avg_slots.len()) as u64;
                    encoder.clear_buffer(&self.avg_result_buffer, 0, Some(total));
                }
                if has_palette {
                    let total = (self.palette_result_stride * palette_slots.len()) as u64;
                    encoder.clear_buffer(&self.palette_result_buffer, 0, Some(total));
                }

                let params_elem_size =
                    NonZeroU64::new(std::mem::size_of::<Params>() as u64).unwrap();
                let avg_elem_size =
                    NonZeroU64::new(std::mem::size_of::<GpuResult>() as u64).unwrap();
                let palette_elem_size =
                    NonZeroU64::new(PALETTE_RESULT_SIZE as u64).unwrap();

                // Dispatch average compute passes
                for &(ps, rs, _, ref p) in &valid_avg {
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("gpu_avg_bg"),
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
                                    offset: (ps * self.params_stride) as u64,
                                    size: Some(params_elem_size),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &self.avg_result_buffer,
                                    offset: (rs * self.avg_result_stride) as u64,
                                    size: Some(avg_elem_size),
                                }),
                            },
                        ],
                    });
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("gpu_average_pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.average_pipeline);
                    pass.set_bind_group(0, &bind_group, &[]);
                    pass.dispatch_workgroups(
                        (p.x1 - p.x0).div_ceil(16),
                        (p.y1 - p.y0).div_ceil(16),
                        1,
                    );
                }

                // Dispatch palette compute passes
                for &(ps, rs, _, ref p) in &valid_palette {
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("gpu_palette_bg"),
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
                                    offset: (ps * self.params_stride) as u64,
                                    size: Some(params_elem_size),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &self.palette_result_buffer,
                                    offset: (rs * self.palette_result_stride) as u64,
                                    size: Some(palette_elem_size),
                                }),
                            },
                        ],
                    });
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("gpu_palette_pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.palette_pipeline);
                    pass.set_bind_group(0, &bind_group, &[]);
                    pass.dispatch_workgroups(
                        (p.x1 - p.x0).div_ceil(16),
                        (p.y1 - p.y0).div_ceil(16),
                        1,
                    );
                }

                // Copy result buffers to readback buffers
                if has_avg {
                    let total = (self.avg_result_stride * avg_slots.len()) as u64;
                    encoder.copy_buffer_to_buffer(
                        &self.avg_result_buffer,
                        0,
                        &self.avg_readback_buffer,
                        0,
                        total,
                    );
                }
                if has_palette {
                    let total = (self.palette_result_stride * palette_slots.len()) as u64;
                    encoder.copy_buffer_to_buffer(
                        &self.palette_result_buffer,
                        0,
                        &self.palette_readback_buffer,
                        0,
                        total,
                    );
                }

                // Single submit
                self.queue.submit(std::iter::once(encoder.finish()));

                // Map readback buffers and poll
                let avg_mapped = if has_avg {
                    let total = (self.avg_result_stride * avg_slots.len()) as u64;
                    let slice = self.avg_readback_buffer.slice(..total);
                    let (sender, receiver) = futures::channel::oneshot::channel();
                    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = sender.send(r); });
                    Some((slice, receiver))
                } else {
                    None
                };

                let palette_mapped = if has_palette {
                    let total = (self.palette_result_stride * palette_slots.len()) as u64;
                    let slice = self.palette_readback_buffer.slice(..total);
                    let (sender, receiver) = futures::channel::oneshot::channel();
                    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = sender.send(r); });
                    Some((slice, receiver))
                } else {
                    None
                };

                let _ = self.device.poll(wgpu::PollType::wait_indefinitely());

                // Read average results
                if let Some((slice, mut receiver)) = avg_mapped {
                    receiver
                        .try_recv()
                        .ok()
                        .flatten()
                        .and_then(|r| r.ok())
                        .ok_or(GpuSamplerError::MapFailed)?;

                    let mapped = slice.get_mapped_range();
                    for &(_, rs, ri, _) in &valid_avg {
                        let offset = rs * self.avg_result_stride;
                        let result: GpuResult = *bytemuck::from_bytes(
                            &mapped[offset..offset + std::mem::size_of::<GpuResult>()],
                        );
                        if result.count > 0 {
                            results[ri] = Some((
                                (result.r_sum / result.count) as u8,
                                (result.g_sum / result.count) as u8,
                                (result.b_sum / result.count) as u8,
                            ));
                        }
                    }
                    drop(mapped);
                    self.avg_readback_buffer.unmap();
                }

                // Read palette results
                if let Some((slice, mut receiver)) = palette_mapped {
                    receiver
                        .try_recv()
                        .ok()
                        .flatten()
                        .and_then(|r| r.ok())
                        .ok_or(GpuSamplerError::MapFailed)?;

                    let mapped = slice.get_mapped_range();
                    for &(_, rs, ri, _) in &valid_palette {
                        let offset = rs * self.palette_result_stride;
                        let bins: &[HistogramBin] = bytemuck::cast_slice(
                            &mapped[offset..offset + PALETTE_RESULT_SIZE],
                        );
                        results[ri] =
                            super::palette::extract_dominant_from_histogram(bins);
                    }
                    drop(mapped);
                    self.palette_readback_buffer.unmap();
                }
            }
        }

        // CPU fallback for unsupported strategies
        if !cpu_indices.is_empty() {
            if let Some(cpu_frame) = frame.convert_to_cpu() {
                for &i in &cpu_indices {
                    let rp = &regions[i];
                    results[i] = super::sample_region(
                        &cpu_frame,
                        rp.x,
                        rp.y,
                        rp.width,
                        rp.height,
                        &rp.strategy,
                    );
                }
            }
        }

        Ok(regions
            .iter()
            .zip(results)
            .map(|(rp, color)| (rp.region_id, color))
            .collect())
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
