use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

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

/// Results from a GPU sampling request, including timing measured on the worker thread.
#[derive(Debug, Clone)]
pub struct SamplingResult {
    pub colors: Vec<(usize, Option<(u8, u8, u8)>)>,
    pub gpu_time_ms: f64,
}

impl Default for SamplingResult {
    fn default() -> Self {
        Self {
            colors: Vec::new(),
            gpu_time_ms: 0.0,
        }
    }
}

struct SamplingRequest {
    frame: Arc<FrameData>,
    regions: Vec<RegionParams>,
    result_tx: tokio::sync::oneshot::Sender<SamplingResult>,
}

/// Handle to a background GPU sampling thread.
///
/// Dropping this struct drops the request channel sender, which causes the
/// background thread's `recv()` to return `Err` and exit cleanly.
pub struct SamplingWorker {
    request_tx: std::sync::mpsc::SyncSender<SamplingRequest>,
    idle: Arc<AtomicBool>,
    _thread: std::thread::JoinHandle<()>,
}

impl SamplingWorker {
    pub fn spawn(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<SamplingRequest>(1);
        let idle = Arc::new(AtomicBool::new(true));
        let idle_flag = idle.clone();

        let thread = std::thread::Builder::new()
            .name("gpu-sampler".to_string())
            .spawn(move || {
                let mut sampler = GpuSampler::new(device, queue);
                while let Ok(request) = request_rx.recv() {
                    idle_flag.store(false, Ordering::Release);
                    let start = Instant::now();
                    let results = sampler.sample_regions(&request.frame, &request.regions);
                    let gpu_time_ms = start.elapsed().as_secs_f64() * 1000.0;
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
                    let _ = request.result_tx.send(SamplingResult {
                        colors,
                        gpu_time_ms,
                    });
                    idle_flag.store(true, Ordering::Release);
                }
                info!("GPU sampler worker thread exiting");
            })
            .expect("failed to spawn gpu-sampler thread");

        Self {
            request_tx,
            idle,
            _thread: thread,
        }
    }

    /// Returns true if the worker is not currently processing a request.
    pub fn is_idle(&self) -> bool {
        self.idle.load(Ordering::Acquire)
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
        map_fn: fn(SamplingResult) -> M,
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

/// A paired result + readback buffer for a single strategy type.
struct ResultBufferPair {
    result: wgpu::Buffer,
    readback: wgpu::Buffer,
    stride: usize,
    capacity: usize,
}

impl ResultBufferPair {
    fn new(device: &wgpu::Device, elem_size: usize, alignment: u32, label: &str) -> Self {
        let stride = aligned_stride(elem_size, alignment);
        let initial_capacity = 1;
        let result = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (stride * initial_capacity) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback_label = format!("{label}_readback");
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&readback_label),
            size: (stride * initial_capacity) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            result,
            readback,
            stride,
            capacity: initial_capacity,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, count: usize, label: &str) {
        if count == 0 || count <= self.capacity {
            return;
        }
        self.result = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (self.stride * count) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback_label = format!("{label}_readback");
        self.readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&readback_label),
            size: (self.stride * count) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.capacity = count;
    }

    fn total_bytes(&self, slot_count: usize) -> u64 {
        (self.stride * slot_count) as u64
    }
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

struct GpuSampler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    average_pipeline: wgpu::ComputePipeline,
    palette_pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    params_stride: usize,
    params_capacity: usize,
    avg_buffers: ResultBufferPair,
    palette_buffers: ResultBufferPair,
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

/// Slot assignment tuple: (params_slot, result_slot, region_index).
type SlotAssignment = (usize, usize, usize);

/// Validated slot with clamped region bounds.
type ValidSlot = (usize, usize, usize, Params);

/// Classification of regions into GPU strategy groups and CPU fallback.
struct ClassifiedRegions {
    avg_slots: Vec<SlotAssignment>,
    palette_slots: Vec<SlotAssignment>,
    cpu_indices: Vec<usize>,
    gpu_count: usize,
}

fn classify_regions(regions: &[RegionParams]) -> ClassifiedRegions {
    let mut avg_slots = Vec::new();
    let mut palette_slots = Vec::new();
    let mut cpu_indices = Vec::new();
    let mut gpu_slot: usize = 0;

    for (i, region) in regions.iter().enumerate() {
        if !region.strategy.supports_gpu() {
            cpu_indices.push(i);
        } else {
            let ps = gpu_slot;
            gpu_slot += 1;
            if region.strategy.id() == super::Palette::ID {
                palette_slots.push((ps, palette_slots.len(), i));
            } else {
                avg_slots.push((ps, avg_slots.len(), i));
            }
        }
    }

    ClassifiedRegions {
        avg_slots,
        palette_slots,
        cpu_indices,
        gpu_count: gpu_slot,
    }
}

fn clamp_region(rp: &RegionParams, width: u32, height: u32) -> Option<Params> {
    let (x0, y0, x1, y1) =
        super::clamp_region_bounds(rp.x, rp.y, rp.width, rp.height, width, height)?;
    Some(Params { x0, y0, x1, y1 })
}

/// Build padded params and collect valid slots for a set of slot assignments.
fn build_valid_slots(
    slots: &[SlotAssignment],
    regions: &[RegionParams],
    width: u32,
    height: u32,
    params_stride: usize,
    padded_params: &mut [u8],
) -> Vec<ValidSlot> {
    let mut valid = Vec::new();
    for &(ps, rs, ri) in slots {
        if let Some(params) = clamp_region(&regions[ri], width, height) {
            let offset = ps * params_stride;
            padded_params[offset..offset + std::mem::size_of::<Params>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
            valid.push((ps, rs, ri, params));
        }
    }
    valid
}

/// Dispatch compute passes for a set of validated slots using a given pipeline.
fn dispatch_passes(
    encoder: &mut wgpu::CommandEncoder,
    device: &wgpu::Device,
    pipeline: &wgpu::ComputePipeline,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    params_buffer: &wgpu::Buffer,
    params_stride: usize,
    buffers: &ResultBufferPair,
    valid_slots: &[ValidSlot],
    params_elem_size: NonZeroU64,
    result_elem_size: NonZeroU64,
    label: &str,
) {
    for &(ps, rs, _, ref p) in valid_slots {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: params_buffer,
                        offset: (ps * params_stride) as u64,
                        size: Some(params_elem_size),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffers.result,
                        offset: (rs * buffers.stride) as u64,
                        size: Some(result_elem_size),
                    }),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups((p.x1 - p.x0).div_ceil(16), (p.y1 - p.y0).div_ceil(16), 1);
    }
}

/// Map a readback buffer and invoke a callback with the mapped data.
/// Handles map_async, poll, error checking, and unmapping.
fn with_readback<F>(
    device: &wgpu::Device,
    buffers: &ResultBufferPair,
    slot_count: usize,
    f: F,
) -> Result<(), GpuSamplerError>
where
    F: FnOnce(&[u8]),
{
    let total = buffers.total_bytes(slot_count);
    let slice = buffers.readback.slice(..total);
    let (sender, mut receiver) = futures::channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = sender.send(r);
    });

    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    receiver
        .try_recv()
        .ok()
        .flatten()
        .and_then(|r| r.ok())
        .ok_or(GpuSamplerError::MapFailed)?;

    let mapped = slice.get_mapped_range();
    f(&mapped);
    drop(mapped);
    buffers.readback.unmap();
    Ok(())
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
            immediate_size: 0,
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

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_sampler_params"),
            size: params_stride as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let avg_buffers = ResultBufferPair::new(
            &device,
            std::mem::size_of::<GpuResult>(),
            limits.min_storage_buffer_offset_alignment,
            "gpu_avg_result",
        );
        let palette_buffers = ResultBufferPair::new(
            &device,
            PALETTE_RESULT_SIZE,
            limits.min_storage_buffer_offset_alignment,
            "gpu_palette_result",
        );

        info!("GPU sampler created");

        Self {
            device,
            queue,
            average_pipeline,
            palette_pipeline,
            bind_group_layout,
            params_buffer,
            params_stride,
            params_capacity: 1,
            avg_buffers,
            palette_buffers,
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

        let classified = classify_regions(regions);

        if classified.gpu_count > 0 {
            self.ensure_params_buffer(classified.gpu_count);
            self.avg_buffers.ensure_capacity(
                &self.device,
                classified.avg_slots.len(),
                "gpu_avg_result",
            );
            self.palette_buffers.ensure_capacity(
                &self.device,
                classified.palette_slots.len(),
                "gpu_palette_result",
            );

            let imported = self.import_frame(frame)?;

            let mut padded_params = vec![0u8; self.params_stride * classified.gpu_count];
            let valid_avg = build_valid_slots(
                &classified.avg_slots,
                regions,
                width,
                height,
                self.params_stride,
                &mut padded_params,
            );
            let valid_palette = build_valid_slots(
                &classified.palette_slots,
                regions,
                width,
                height,
                self.params_stride,
                &mut padded_params,
            );

            let has_avg = !valid_avg.is_empty();
            let has_palette = !valid_palette.is_empty();

            if has_avg || has_palette {
                self.queue.write_buffer(
                    &self.params_buffer,
                    0,
                    &padded_params[..self.params_stride * classified.gpu_count],
                );

                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("gpu_sampler_batch"),
                        });

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

                if has_avg {
                    let total = self.avg_buffers.total_bytes(classified.avg_slots.len());
                    encoder.clear_buffer(&self.avg_buffers.result, 0, Some(total));
                }
                if has_palette {
                    let total = self
                        .palette_buffers
                        .total_bytes(classified.palette_slots.len());
                    encoder.clear_buffer(&self.palette_buffers.result, 0, Some(total));
                }

                let params_elem_size =
                    NonZeroU64::new(std::mem::size_of::<Params>() as u64).unwrap();
                let avg_elem_size =
                    NonZeroU64::new(std::mem::size_of::<GpuResult>() as u64).unwrap();
                let palette_elem_size = NonZeroU64::new(PALETTE_RESULT_SIZE as u64).unwrap();

                dispatch_passes(
                    &mut encoder,
                    &self.device,
                    &self.average_pipeline,
                    &self.bind_group_layout,
                    &imported.view,
                    &self.params_buffer,
                    self.params_stride,
                    &self.avg_buffers,
                    &valid_avg,
                    params_elem_size,
                    avg_elem_size,
                    "gpu_average_pass",
                );

                dispatch_passes(
                    &mut encoder,
                    &self.device,
                    &self.palette_pipeline,
                    &self.bind_group_layout,
                    &imported.view,
                    &self.params_buffer,
                    self.params_stride,
                    &self.palette_buffers,
                    &valid_palette,
                    params_elem_size,
                    palette_elem_size,
                    "gpu_palette_pass",
                );

                if has_avg {
                    let total = self.avg_buffers.total_bytes(classified.avg_slots.len());
                    encoder.copy_buffer_to_buffer(
                        &self.avg_buffers.result,
                        0,
                        &self.avg_buffers.readback,
                        0,
                        total,
                    );
                }
                if has_palette {
                    let total = self
                        .palette_buffers
                        .total_bytes(classified.palette_slots.len());
                    encoder.copy_buffer_to_buffer(
                        &self.palette_buffers.result,
                        0,
                        &self.palette_buffers.readback,
                        0,
                        total,
                    );
                }

                self.queue.submit(std::iter::once(encoder.finish()));

                if has_avg {
                    with_readback(
                        &self.device,
                        &self.avg_buffers,
                        classified.avg_slots.len(),
                        |mapped| {
                            for &(_, rs, ri, _) in &valid_avg {
                                let offset = rs * self.avg_buffers.stride;
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
                        },
                    )?;
                }

                if has_palette {
                    with_readback(
                        &self.device,
                        &self.palette_buffers,
                        classified.palette_slots.len(),
                        |mapped| {
                            for &(_, rs, ri, _) in &valid_palette {
                                let offset = rs * self.palette_buffers.stride;
                                let bins: &[HistogramBin] = bytemuck::cast_slice(
                                    &mapped[offset..offset + PALETTE_RESULT_SIZE],
                                );
                                results[ri] = super::palette::extract_dominant_from_histogram(bins);
                            }
                        },
                    )?;
                }
            }
        }

        // CPU fallback for unsupported strategies
        if !classified.cpu_indices.is_empty() {
            if let Some(cpu_frame) = frame.convert_to_cpu() {
                for &i in &classified.cpu_indices {
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
            #[cfg(target_os = "macos")]
            FrameData::IOSurface {
                surface,
                width,
                height,
            } => {
                // Re-import on the sampler thread (safe — not inside winit event handler).
                // We need an owned wgpu::Texture for PendingCopy.
                // Wrap in autoreleasepool to prevent ObjC object leaks from Metal calls.
                let (imported, wgpu_format) = screencapturekit::metal::autoreleasepool(|| unsafe {
                    crate::platform::macos::metal_import::import_iosurface_texture(
                        &self.device,
                        surface,
                        *width,
                        *height,
                    )
                })
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
                frame,
                width,
                height,
            } => {
                use windows::Win32::Foundation::HANDLE;
                let handle = HANDLE(frame.shared_handle().0 as *mut core::ffi::c_void);
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
                let format = wgpu::TextureFormat::Bgra8UnormSrgb;
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
            let non_srgb = crate::texture_format::non_srgb_equivalent(format);
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
    let view_format = crate::texture_format::non_srgb_equivalent(original_format);
    texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(view_format),
        ..Default::default()
    })
}
