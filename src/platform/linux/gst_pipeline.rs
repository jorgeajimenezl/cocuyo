use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_allocators::{DmaBufAllocator, FdMemoryFlags};
use gstreamer_app as gst_app;
use pipewire::spa;
use std::os::fd::RawFd;
use tracing::{info, warn};

use super::formats::to_gst_format;

/// Available GPU backends for video processing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GpuBackend {
    /// Automatically select the best backend based on the compositor's GPU.
    Auto,
    Cuda(CudaDevice),
    OpenGL,
    Cpu,
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Auto => write!(f, "Auto"),
            GpuBackend::Cuda(dev) => write!(f, "{}", dev.name),
            GpuBackend::OpenGL => write!(f, "OpenGL (GPU)"),
            GpuBackend::Cpu => write!(f, "CPU (Software)"),
        }
    }
}

impl GpuBackend {
    pub fn config_key(&self) -> String {
        match self {
            GpuBackend::Auto => "auto".into(),
            GpuBackend::Cuda(dev) => format!("cuda:{}", dev.index),
            GpuBackend::OpenGL => "opengl".into(),
            GpuBackend::Cpu => "cpu".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CudaDevice {
    pub index: i32,
    pub name: String,
}

#[derive(Debug)]
pub enum GstError {
    InitError(String),
    PipelineError(String),
    ConversionError(String),
}

impl std::fmt::Display for GstError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GstError::InitError(msg) => write!(f, "GStreamer init error: {}", msg),
            GstError::PipelineError(msg) => write!(f, "Pipeline error: {}", msg),
            GstError::ConversionError(msg) => write!(f, "Conversion error: {}", msg),
        }
    }
}

impl std::error::Error for GstError {}

/// Detects all available GPU backends on the system.
pub fn detect_available_backends() -> Vec<GpuBackend> {
    let mut backends = Vec::new();

    if let Some(cuda) = detect_cuda_device() {
        backends.push(GpuBackend::Cuda(cuda));
    }

    if detect_opengl_available() {
        backends.push(GpuBackend::OpenGL);
    }

    backends.push(GpuBackend::Cpu);
    backends
}

fn detect_cuda_device() -> Option<CudaDevice> {
    gst::ElementFactory::find("cudaconvert")?;
    gst::ElementFactory::find("cudaupload")?;
    gst::ElementFactory::find("cudadownload")?;

    let element = gst::ElementFactory::make("cudaconvert").build().ok()?;
    let gpu_name = get_nvidia_gpu_name().unwrap_or_else(|| "NVIDIA GPU".to_string());

    drop(element);
    info!(gpu = %gpu_name, "CUDA support detected");

    Some(CudaDevice {
        index: 0,
        name: format!("{} (CUDA)", gpu_name),
    })
}

fn get_nvidia_gpu_name() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_str()?;
        if !name.starts_with("renderD") {
            continue;
        }

        let vendor = std::fs::read_to_string(path.join("device/vendor"))
            .ok()?
            .trim()
            .to_lowercase();

        if vendor == "0x10de" {
            return Some("NVIDIA GPU".to_string());
        }
    }
    None
}

fn detect_opengl_available() -> bool {
    let available = gst::ElementFactory::find("glcolorconvert")
        .and_then(|factory| factory.create().build().ok())
        .is_some();

    if available {
        info!("OpenGL colorspace conversion available");
    }

    available
}

pub struct GstVideoConverter {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    dmabuf_allocator: Option<DmaBufAllocator>,
    backend: GpuBackend,
}

impl GstVideoConverter {
    pub fn new(
        width: u32,
        height: u32,
        format: spa::param::video::VideoFormat,
        backend: GpuBackend,
    ) -> Result<Self, GstError> {
        let format_str = to_gst_format(format)
            .ok_or_else(|| GstError::InitError(format!("Unsupported format: {:?}", format)))?;

        let pipeline = gst::Pipeline::new();

        let appsrc = gst::ElementFactory::make("appsrc")
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create appsrc: {}", e)))?
            .downcast::<gst_app::AppSrc>()
            .unwrap();

        let appsink = gst::ElementFactory::make("appsink")
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create appsink: {}", e)))?
            .downcast::<gst_app::AppSink>()
            .unwrap();

        let input_caps = gst::Caps::builder("video/x-raw")
            .field("format", format_str)
            .field("width", width as i32)
            .field("height", height as i32)
            .build();

        appsrc.set_caps(Some(&input_caps));
        appsrc.set_format(gst::Format::Time);
        appsrc.set_property("block", true);

        let output_caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", width as i32)
            .field("height", height as i32)
            .build();

        appsink.set_caps(Some(&output_caps));
        appsink.set_property("emit-signals", false);
        appsink.set_property("sync", false);

        let actual_backend = Self::build_pipeline(&pipeline, &appsrc, &appsink, &backend)?;

        if actual_backend != backend {
            warn!(
                requested = %backend,
                actual = %actual_backend,
                "Requested backend unavailable, using fallback"
            );
        }

        info!(backend = %actual_backend, "Video converter initialized");

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| GstError::PipelineError(format!("Failed to start pipeline: {:?}", e)))?;

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
            dmabuf_allocator: None,
            backend: actual_backend,
        })
    }

    fn build_pipeline(
        pipeline: &gst::Pipeline,
        appsrc: &gst_app::AppSrc,
        appsink: &gst_app::AppSink,
        backend: &GpuBackend,
    ) -> Result<GpuBackend, GstError> {
        match backend {
            GpuBackend::Auto => {
                unreachable!("Auto backend must be resolved before creating GstVideoConverter")
            }
            GpuBackend::Cuda(device) => {
                match Self::try_build_cuda_pipeline(pipeline, appsrc, appsink, device.index) {
                    Ok(()) => Ok(backend.clone()),
                    Err(e) => {
                        warn!(error = %e, "Failed to create CUDA pipeline, falling back to CPU");
                        Self::build_cpu_pipeline(pipeline, appsrc, appsink)?;
                        Ok(GpuBackend::Cpu)
                    }
                }
            }
            GpuBackend::OpenGL => {
                match Self::try_build_opengl_pipeline(pipeline, appsrc, appsink) {
                    Ok(()) => Ok(backend.clone()),
                    Err(e) => {
                        warn!(error = %e, "Failed to create OpenGL pipeline, falling back to CPU");
                        Self::build_cpu_pipeline(pipeline, appsrc, appsink)?;
                        Ok(GpuBackend::Cpu)
                    }
                }
            }
            GpuBackend::Cpu => {
                Self::build_cpu_pipeline(pipeline, appsrc, appsink)?;
                Ok(GpuBackend::Cpu)
            }
        }
    }

    fn try_build_cuda_pipeline(
        pipeline: &gst::Pipeline,
        appsrc: &gst_app::AppSrc,
        appsink: &gst_app::AppSink,
        device_id: i32,
    ) -> Result<(), GstError> {
        let cudaupload = gst::ElementFactory::make("cudaupload")
            .property("cuda-device-id", device_id)
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create cudaupload: {}", e)))?;

        let cudaconvert = gst::ElementFactory::make("cudaconvert")
            .property("cuda-device-id", device_id)
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create cudaconvert: {}", e)))?;

        let cudadownload = make_element("cudadownload")?;

        build_pipeline_with_elements(
            pipeline, appsrc, appsink,
            &[&cudaupload, &cudaconvert, &cudadownload],
            "CUDA",
        )?;

        info!(device_id, "CUDA pipeline created");
        Ok(())
    }

    fn try_build_opengl_pipeline(
        pipeline: &gst::Pipeline,
        appsrc: &gst_app::AppSrc,
        appsink: &gst_app::AppSink,
    ) -> Result<(), GstError> {
        let glupload = make_element("glupload")?;
        let glcolorconvert = make_element("glcolorconvert")?;
        let gldownload = make_element("gldownload")?;

        build_pipeline_with_elements(
            pipeline, appsrc, appsink,
            &[&glupload, &glcolorconvert, &gldownload],
            "OpenGL",
        )?;

        info!("OpenGL pipeline created");
        Ok(())
    }

    fn build_cpu_pipeline(
        pipeline: &gst::Pipeline,
        appsrc: &gst_app::AppSrc,
        appsink: &gst_app::AppSink,
    ) -> Result<(), GstError> {
        let videoconvert = make_element("videoconvert")?;

        build_pipeline_with_elements(
            pipeline, appsrc, appsink,
            &[&videoconvert],
            "CPU",
        )?;

        info!("CPU pipeline created");
        Ok(())
    }

    pub fn backend(&self) -> &GpuBackend {
        &self.backend
    }

    pub fn push_buffer(&self, data: &[u8]) -> Result<(), GstError> {
        let mut buffer = gst::Buffer::with_size(data.len())
            .map_err(|e| GstError::ConversionError(format!("Failed to create buffer: {}", e)))?;

        {
            let buffer_ref = buffer.get_mut().unwrap();
            let mut map = buffer_ref
                .map_writable()
                .map_err(|e| GstError::ConversionError(format!("Failed to map buffer: {}", e)))?;
            map.copy_from_slice(data);
        }

        self.appsrc
            .push_buffer(buffer)
            .map_err(|e| GstError::ConversionError(format!("Failed to push buffer: {:?}", e)))?;

        Ok(())
    }

    pub fn push_dmabuf(&mut self, fd: RawFd, size: usize) -> Result<(), GstError> {
        let allocator = self.dmabuf_allocator.get_or_insert_with(DmaBufAllocator::new);

        let memory = unsafe { allocator.alloc_with_flags(fd, size, FdMemoryFlags::DONT_CLOSE) }
            .map_err(|e| {
                GstError::ConversionError(format!("Failed to allocate DMA-BUF memory: {}", e))
            })?;

        let mut buffer = gst::Buffer::new();
        {
            let buffer_ref = buffer.get_mut().unwrap();
            buffer_ref.append_memory(memory);
        }

        self.appsrc.push_buffer(buffer).map_err(|e| {
            GstError::ConversionError(format!("Failed to push DMA-BUF buffer: {:?}", e))
        })?;

        Ok(())
    }

    pub fn pull_rgba_frame(&self) -> Result<Vec<u8>, GstError> {
        let sample = self
            .appsink
            .pull_sample()
            .map_err(|e| GstError::ConversionError(format!("Failed to pull sample: {:?}", e)))?;

        let buffer = sample
            .buffer()
            .ok_or_else(|| GstError::ConversionError("No buffer in sample".to_string()))?;

        let map = buffer
            .map_readable()
            .map_err(|e| GstError::ConversionError(format!("Failed to map buffer: {}", e)))?;

        Ok(map.as_slice().to_vec())
    }
}

impl Drop for GstVideoConverter {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn make_element(name: &str) -> Result<gst::Element, GstError> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|e| GstError::PipelineError(format!("Failed to create {}: {}", name, e)))
}

fn build_pipeline_with_elements(
    pipeline: &gst::Pipeline,
    appsrc: &gst_app::AppSrc,
    appsink: &gst_app::AppSink,
    elements: &[&gst::Element],
    label: &str,
) -> Result<(), GstError> {
    let mut all: Vec<&gst::Element> = Vec::with_capacity(elements.len() + 2);
    all.push(appsrc.upcast_ref());
    all.extend_from_slice(elements);
    all.push(appsink.upcast_ref());

    pipeline
        .add_many(all.iter().copied())
        .map_err(|e| GstError::PipelineError(format!("Failed to add {} elements: {}", label, e)))?;

    gst::Element::link_many(all.iter().copied())
        .map_err(|e| GstError::PipelineError(format!("Failed to link {} elements: {}", label, e)))?;

    Ok(())
}

/// Resolves the Auto backend selection to a concrete backend.
///
/// For DMA-BUF frames, reads `/proc/self/fdinfo/<fd>` to detect the DRM driver
/// and selects a matching backend. For CPU frames (`dmabuf_fd` is `None`), returns
/// the best available backend (CUDA > OpenGL > CPU).
pub fn resolve_auto_backend(dmabuf_fd: Option<RawFd>, available: &[GpuBackend]) -> GpuBackend {
    if let Some(fd) = dmabuf_fd {
        if let Some(driver) = read_drm_driver(fd) {
            info!(driver = %driver, "Detected DRM driver from DMA-BUF fd");

            if driver.contains("nvidia") {
                if let Some(cuda) = available.iter().find(|b| matches!(b, GpuBackend::Cuda(_))) {
                    return cuda.clone();
                }
            }

            // Non-NVIDIA driver (amdgpu, i915, xe, etc.) → prefer OpenGL
            if available.iter().any(|b| matches!(b, GpuBackend::OpenGL)) {
                return GpuBackend::OpenGL;
            }
        }
    }

    // Fallback: best available (CUDA > OpenGL > CPU)
    best_available_backend(available)
}

fn best_available_backend(available: &[GpuBackend]) -> GpuBackend {
    for backend in available {
        if matches!(backend, GpuBackend::Cuda(_)) {
            return backend.clone();
        }
    }
    if available.iter().any(|b| matches!(b, GpuBackend::OpenGL)) {
        return GpuBackend::OpenGL;
    }
    GpuBackend::Cpu
}

fn read_drm_driver(fd: RawFd) -> Option<String> {
    let path = format!("/proc/self/fdinfo/{}", fd);
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(driver) = line.strip_prefix("drm-driver:") {
            return Some(driver.trim().to_lowercase());
        }
    }
    None
}
