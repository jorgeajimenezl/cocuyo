#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::sync::Arc;

#[cfg(target_os = "linux")]
use drm_fourcc::DrmFourcc;
use iced::widget::shader;
use iced::{Rectangle, mouse};
use tracing::{error, warn};

use crate::frame::FrameData;
#[cfg(target_os = "linux")]
use crate::platform::linux::vulkan_dmabuf;
#[cfg(target_os = "macos")]
use crate::platform::macos::metal_import;
#[cfg(target_os = "windows")]
use crate::platform::windows::dx12_import;

/// Scene data passed to the shader widget each frame.
pub struct VideoScene {
    frame: Option<FrameInfo>,
}

/// Extracted frame information for the shader primitive.
enum FrameInfo {
    #[cfg(target_os = "linux")]
    DmaBuf {
        fd: std::os::fd::RawFd,
        width: u32,
        height: u32,
        drm_format: DrmFourcc,
        stride: u32,
        offset: u32,
    },
    #[cfg(target_os = "macos")]
    IOSurface {
        surface: screencapturekit::cm::IOSurface,
        width: u32,
        height: u32,
    },
    #[cfg(target_os = "windows")]
    D3DShared {
        shared_handle: isize,
        width: u32,
        height: u32,
    },
    Cpu {
        data: Arc<Vec<u8>>,
        width: u32,
        height: u32,
    },
}

impl VideoScene {
    pub fn new(frame: Option<&FrameData>) -> Self {
        let frame = frame.map(|f| match f {
            #[cfg(target_os = "linux")]
            FrameData::DmaBuf {
                fd,
                width,
                height,
                drm_format,
                stride,
                offset,
                ..
            } => FrameInfo::DmaBuf {
                fd: fd.as_raw_fd(),
                width: *width,
                height: *height,
                drm_format: *drm_format,
                stride: *stride,
                offset: *offset,
            },
            #[cfg(target_os = "macos")]
            FrameData::IOSurface {
                surface,
                width,
                height,
            } => FrameInfo::IOSurface {
                surface: surface.clone(),
                width: *width,
                height: *height,
            },
            #[cfg(target_os = "windows")]
            FrameData::D3DShared {
                frame,
                width,
                height,
            } => FrameInfo::D3DShared {
                shared_handle: frame.shared_handle().0 as isize,
                width: *width,
                height: *height,
            },
            FrameData::Cpu {
                data,
                width,
                height,
            } => FrameInfo::Cpu {
                data: Arc::clone(data),
                width: *width,
                height: *height,
            },
        });

        Self { frame }
    }
}

impl<Message> shader::Program<Message> for VideoScene {
    type State = ();
    type Primitive = VideoPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        match &self.frame {
            #[cfg(target_os = "linux")]
            Some(FrameInfo::DmaBuf {
                fd,
                width,
                height,
                drm_format,
                stride,
                offset,
            }) => VideoPrimitive::DmaBuf {
                fd: *fd,
                width: *width,
                height: *height,
                drm_format: *drm_format,
                stride: *stride,
                offset: *offset,
                bounds,
            },
            #[cfg(target_os = "macos")]
            Some(FrameInfo::IOSurface {
                surface,
                width,
                height,
            }) => VideoPrimitive::IOSurface {
                surface: surface.clone(),
                width: *width,
                height: *height,
                bounds,
            },
            #[cfg(target_os = "windows")]
            Some(FrameInfo::D3DShared {
                shared_handle,
                width,
                height,
            }) => VideoPrimitive::D3DShared {
                shared_handle: *shared_handle,
                width: *width,
                height: *height,
                bounds,
            },
            Some(FrameInfo::Cpu {
                data,
                width,
                height,
            }) => VideoPrimitive::Cpu {
                data: Arc::clone(data),
                width: *width,
                height: *height,
                bounds,
            },
            None => VideoPrimitive::Empty,
        }
    }
}

/// Primitive that carries per-frame data to the GPU pipeline.
#[derive(Debug)]
pub enum VideoPrimitive {
    #[cfg(target_os = "linux")]
    DmaBuf {
        fd: std::os::fd::RawFd,
        width: u32,
        height: u32,
        drm_format: DrmFourcc,
        stride: u32,
        offset: u32,
        bounds: Rectangle,
    },
    #[cfg(target_os = "macos")]
    IOSurface {
        surface: screencapturekit::cm::IOSurface,
        width: u32,
        height: u32,
        bounds: Rectangle,
    },
    #[cfg(target_os = "windows")]
    D3DShared {
        shared_handle: isize,
        width: u32,
        height: u32,
        bounds: Rectangle,
    },
    Cpu {
        data: Arc<Vec<u8>>,
        width: u32,
        height: u32,
        bounds: Rectangle,
    },
    Empty,
}

impl shader::Primitive for VideoPrimitive {
    type Pipeline = VideoPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &iced::advanced::graphics::Viewport,
    ) {
        match self {
            #[cfg(target_os = "linux")]
            VideoPrimitive::DmaBuf {
                fd,
                width,
                height,
                drm_format,
                stride,
                offset,
                bounds,
            } => {
                pipeline.prepare_dmabuf(
                    device,
                    queue,
                    *fd,
                    *width,
                    *height,
                    *drm_format,
                    *stride,
                    *offset,
                    *bounds,
                );
            }
            #[cfg(target_os = "macos")]
            VideoPrimitive::IOSurface {
                surface,
                width,
                height,
                bounds,
            } => {
                pipeline.prepare_iosurface(
                    device,
                    queue,
                    surface,
                    *width,
                    *height,
                    *bounds,
                );
            }
            #[cfg(target_os = "windows")]
            VideoPrimitive::D3DShared {
                shared_handle,
                width,
                height,
                bounds,
            } => {
                pipeline.prepare_d3d_shared(
                    device,
                    queue,
                    *shared_handle,
                    *width,
                    *height,
                    *bounds,
                );
            }
            VideoPrimitive::Cpu {
                data,
                width,
                height,
                bounds,
            } => {
                pipeline.prepare_cpu(device, queue, data, *width, *height, *bounds);
            }
            VideoPrimitive::Empty => {
                pipeline.current_bind_group = None;
            }
        }
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        pipeline.render(encoder, target, clip_bounds);
    }
}

/// Cached GPU texture reused across frames when dimensions and format are unchanged.
struct CachedTexture {
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

/// GPU pipeline that manages the render pipeline and textures.
pub struct VideoPipeline {
    render_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    current_bind_group: Option<wgpu::BindGroup>,
    cached_texture: Option<CachedTexture>,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    // Scale and offset for aspect-ratio-correct rendering
    scale: [f32; 2],
    offset: [f32; 2],
}

impl shader::Pipeline for VideoPipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        crate::gpu_context::set_gpu_context(device.clone(), queue.clone());

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("video_shader.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video_bind_group_layout"),
            entries: &[
                // Texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            std::num::NonZero::new(std::mem::size_of::<Uniforms>() as u64).unwrap(),
                        ),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("video_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video_uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            render_pipeline,
            bind_group_layout,
            sampler,
            uniform_buffer,
            current_bind_group: None,
            cached_texture: None,
        }
    }
}

impl VideoPipeline {
    fn get_or_create_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> &wgpu::Texture {
        let needs_recreate = match &self.cached_texture {
            Some(ct) => ct.width != width || ct.height != height || ct.format != format,
            None => true,
        };

        if needs_recreate {
            self.cached_texture = Some(CachedTexture {
                texture: device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("video_frame"),
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
                    view_formats: &[],
                }),
                width,
                height,
                format,
            });
        }

        &self.cached_texture.as_ref().unwrap().texture
    }

    #[cfg(target_os = "linux")]
    fn prepare_dmabuf(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fd: std::os::fd::RawFd,
        width: u32,
        height: u32,
        drm_format: DrmFourcc,
        stride: u32,
        offset: u32,
        bounds: Rectangle,
    ) {
        let result = unsafe {
            vulkan_dmabuf::import_dmabuf_texture(
                device, fd, width, height, drm_format, stride, offset,
            )
        };

        match result {
            Ok((texture, _wgpu_format)) => {
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.update_bind_group(device, queue, &view, width, height, bounds);
            }
            Err(e) => {
                error!(
                    error = %e,
                    fd,
                    width,
                    height,
                    format = ?drm_format,
                    "DMA-BUF Vulkan import failed, disabling for future frames"
                );
                vulkan_dmabuf::mark_dmabuf_import_failed();
                self.current_bind_group = None;
                self.cached_texture = None;
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn prepare_d3d_shared(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shared_handle: isize,
        width: u32,
        height: u32,
        bounds: Rectangle,
    ) {
        use windows::Win32::Foundation::HANDLE;

        let handle = HANDLE(shared_handle as *mut core::ffi::c_void);
        let result = unsafe { dx12_import::import_shared_texture(device, handle, width, height) };

        match result {
            Ok((texture, _wgpu_format)) => {
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.update_bind_group(device, queue, &view, width, height, bounds);
            }
            Err(e) => {
                error!(
                    error = %e,
                    width,
                    height,
                    "D3D shared texture import failed, disabling for future frames"
                );
                dx12_import::mark_d3d_shared_import_failed();
                self.current_bind_group = None;
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn prepare_iosurface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &screencapturekit::cm::IOSurface,
        width: u32,
        height: u32,
        bounds: Rectangle,
    ) {
        // Wrap Metal/ObjC calls in an autoreleasepool to prevent Cocoa
        // run-loop re-entrancy panics inside the winit event handler.
        let result = screencapturekit::metal::autoreleasepool(|| unsafe {
            metal_import::import_iosurface_texture(device, surface, width, height)
        });

        match result {
            Ok((texture, _wgpu_format)) => {
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.update_bind_group(device, queue, &view, width, height, bounds);
            }
            Err(e) => {
                error!(
                    error = %e,
                    width,
                    height,
                    "IOSurface Metal import failed, disabling for future frames"
                );
                metal_import::mark_iosurface_import_failed();
                self.current_bind_group = None;
            }
        }
    }

    fn prepare_cpu(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        width: u32,
        height: u32,
        bounds: Rectangle,
    ) {
        let expected_size = (width * height * 4) as usize;
        if data.len() < expected_size {
            warn!(
                data_len = data.len(),
                expected = expected_size,
                "CPU frame data too small"
            );
            self.current_bind_group = None;
            self.cached_texture = None;
            return;
        }

        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let texture = self.get_or_create_texture(device, width, height, format);

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data[..expected_size],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let view = self
            .cached_texture
            .as_ref()
            .unwrap()
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.update_bind_group(device, queue, &view, width, height, bounds);
    }

    fn update_bind_group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_view: &wgpu::TextureView,
        frame_width: u32,
        frame_height: u32,
        bounds: Rectangle,
    ) {
        let layout = crate::region::ContainLayout::compute(frame_width, frame_height, bounds);

        let uniforms = Uniforms {
            scale: [layout.scale_x, layout.scale_y],
            offset: [(1.0 - layout.scale_x) * 0.5, (1.0 - layout.scale_y) * 0.5],
        };

        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        self.current_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        }));
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(bind_group) = &self.current_bind_group else {
            return;
        };

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        );
        render_pass.set_viewport(
            clip_bounds.x as f32,
            clip_bounds.y as f32,
            clip_bounds.width as f32,
            clip_bounds.height as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
