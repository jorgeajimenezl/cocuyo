use std::os::fd::AsRawFd;

use drm_fourcc::DrmFourcc;
use iced::widget::shader;
use iced::{Rectangle, mouse};
use tracing::{error, warn};

use crate::frame::FrameData;
use crate::platform::linux::vulkan_dmabuf;

/// Scene data passed to the shader widget each frame.
pub struct VideoScene {
    frame: Option<FrameInfo>,
}

/// Extracted frame information for the shader primitive.
enum FrameInfo {
    DmaBuf {
        fd: std::os::fd::RawFd,
        width: u32,
        height: u32,
        drm_format: DrmFourcc,
        stride: u32,
    },
    Cpu {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
}

impl VideoScene {
    pub fn new(frame: Option<&FrameData>) -> Self {
        let frame = frame.map(|f| match f {
            FrameData::DmaBuf {
                fd,
                width,
                height,
                drm_format,
                stride,
                ..
            } => FrameInfo::DmaBuf {
                fd: fd.as_raw_fd(),
                width: *width,
                height: *height,
                drm_format: *drm_format,
                stride: *stride,
            },
            FrameData::Cpu {
                data,
                width,
                height,
            } => FrameInfo::Cpu {
                data: data.clone(),
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
            Some(FrameInfo::DmaBuf {
                fd,
                width,
                height,
                drm_format,
                stride,
            }) => VideoPrimitive::DmaBuf {
                fd: *fd,
                width: *width,
                height: *height,
                drm_format: *drm_format,
                stride: *stride,
                bounds,
            },
            Some(FrameInfo::Cpu {
                data,
                width,
                height,
            }) => VideoPrimitive::Cpu {
                data: data.clone(),
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
    DmaBuf {
        fd: std::os::fd::RawFd,
        width: u32,
        height: u32,
        drm_format: DrmFourcc,
        stride: u32,
        bounds: Rectangle,
    },
    Cpu {
        data: Vec<u8>,
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
            VideoPrimitive::DmaBuf {
                fd,
                width,
                height,
                drm_format,
                stride,
                bounds,
            } => {
                pipeline.prepare_dmabuf(device, *fd, *width, *height, *drm_format, *stride, *bounds);
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

/// GPU pipeline that manages the render pipeline and textures.
pub struct VideoPipeline {
    render_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    current_bind_group: Option<wgpu::BindGroup>,
    // Keep the imported texture alive until the next frame replaces it
    _current_texture: Option<wgpu::Texture>,
    #[allow(dead_code)]
    target_format: wgpu::TextureFormat,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    // Scale and offset for aspect-ratio-correct rendering
    scale: [f32; 2],
    offset: [f32; 2],
}

impl shader::Pipeline for VideoPipeline {
    fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
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
            _current_texture: None,
            target_format: format,
        }
    }
}

impl VideoPipeline {
    fn prepare_dmabuf(
        &mut self,
        device: &wgpu::Device,
        fd: std::os::fd::RawFd,
        width: u32,
        height: u32,
        drm_format: DrmFourcc,
        stride: u32,
        bounds: Rectangle,
    ) {
        let result = unsafe {
            vulkan_dmabuf::import_dmabuf_texture(device, fd, width, height, drm_format, stride)
        };

        match result {
            Ok((texture, _wgpu_format)) => {
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.update_bind_group(device, &view, width, height, bounds);
                self._current_texture = Some(texture);
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
                self._current_texture = None;
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
            self._current_texture = None;
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cpu_frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
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

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.update_bind_group(device, &view, width, height, bounds);
        self._current_texture = Some(texture);
    }

    fn update_bind_group(
        &mut self,
        device: &wgpu::Device,
        texture_view: &wgpu::TextureView,
        frame_width: u32,
        frame_height: u32,
        bounds: Rectangle,
    ) {
        // Compute aspect-ratio-correct scale (ContentFit::Contain)
        let frame_aspect = frame_width as f32 / frame_height as f32;
        let bounds_aspect = bounds.width / bounds.height;

        let (scale_x, scale_y) = if frame_aspect > bounds_aspect {
            // Frame is wider than bounds: fit to width
            (1.0, bounds_aspect / frame_aspect)
        } else {
            // Frame is taller than bounds: fit to height
            (frame_aspect / bounds_aspect, 1.0)
        };

        let uniforms = Uniforms {
            scale: [scale_x, scale_y],
            offset: [(1.0 - scale_x) * 0.5, (1.0 - scale_y) * 0.5],
        };

        // Write uniforms - we'll update via a new buffer write
        // Since we can't write to the buffer here (no queue), we recreate the buffer
        // Actually we need queue for write_buffer... Let's use mapped_at_creation approach
        // or just include queue in our calls. For now, use a new buffer each frame.
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video_uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });

        {
            let mut mapping = uniform_buffer.slice(..).get_mapped_range_mut();
            mapping.copy_from_slice(bytemuck::bytes_of(&uniforms));
        }
        uniform_buffer.unmap();
        self.uniform_buffer = uniform_buffer;

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
