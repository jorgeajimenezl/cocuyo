use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_allocators::{DmaBufAllocator, FdMemoryFlags};
use gstreamer_app as gst_app;
use pipewire::spa;
use std::os::fd::RawFd;
use tracing::{info, warn};

/// Represents available GPU backends for video processing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuBackend {
    /// NVIDIA CUDA acceleration (requires gst-plugins-bad with CUDA support)
    Cuda(CudaDevice),
    /// VA-API acceleration (Intel/AMD, requires gstreamer-vaapi)
    Vaapi(VaapiDevice),
    /// Software (CPU) processing
    Cpu,
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Cuda(dev) => write!(f, "{}", dev.name),
            GpuBackend::Vaapi(dev) => write!(f, "{}", dev.name),
            GpuBackend::Cpu => write!(f, "CPU (Software)"),
        }
    }
}

/// CUDA device information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CudaDevice {
    pub index: i32,
    pub name: String,
}

/// VA-API device information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaapiDevice {
    pub path: String,
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

/// Detect all available GPU backends on the system
pub fn detect_available_backends() -> Vec<GpuBackend> {
    let mut backends = Vec::new();

    // Check for CUDA (NVIDIA) support
    if let Some(cuda_device) = detect_cuda_device() {
        backends.push(GpuBackend::Cuda(cuda_device));
    }

    // Check for OpenGL support (works on Intel/AMD/NVIDIA)
    if let Some(vaapi_devices) = detect_opengl_devices() {
        for device in vaapi_devices {
            backends.push(GpuBackend::Vaapi(device));
        }
    }

    // CPU is always available
    backends.push(GpuBackend::Cpu);

    backends
}

/// Detect NVIDIA CUDA GPU
fn detect_cuda_device() -> Option<CudaDevice> {
    // Check if cudaconvert element is available
    gst::ElementFactory::find("cudaconvert")?;
    gst::ElementFactory::find("cudaupload")?;
    gst::ElementFactory::find("cudadownload")?;

    // Try to create the element to verify CUDA works
    let element = gst::ElementFactory::make("cudaconvert").build().ok()?;

    // Get the GPU name if possible
    let gpu_name = get_nvidia_gpu_name().unwrap_or_else(|| "NVIDIA GPU".to_string());

    drop(element);

    info!(gpu = %gpu_name, "CUDA support detected");

    Some(CudaDevice {
        index: 0,
        name: format!("{} (CUDA)", gpu_name),
    })
}

/// Get NVIDIA GPU name from sysfs
fn get_nvidia_gpu_name() -> Option<String> {
    // Try to read from sysfs first
    for entry in std::fs::read_dir("/sys/class/drm").ok()? {
        let entry = entry.ok()?;
        let path = entry.path();

        // Look for card* directories
        let name = path.file_name()?.to_str()?;
        if !name.starts_with("card") || name.contains("render") {
            continue;
        }

        // Check vendor
        let vendor_path = path.join("device/vendor");
        let vendor = std::fs::read_to_string(&vendor_path).ok()?;
        if vendor.trim() != "0x10de" {
            continue; // Not NVIDIA
        }

        // Found NVIDIA device
        return Some("NVIDIA GPU".to_string());
    }

    None
}

/// Detect OpenGL-capable devices for GPU colorspace conversion
fn detect_opengl_devices() -> Option<Vec<VaapiDevice>> {
    // Check if glcolorconvert is available (OpenGL-based conversion)
    let factory = gst::ElementFactory::find("glcolorconvert")?;
    let _element = factory.create().build().ok()?;

    info!("OpenGL colorspace conversion available");

    // Return a single OpenGL device option
    Some(vec![VaapiDevice {
        path: "opengl".to_string(),
        name: "OpenGL (GPU)".to_string(),
    }])
}

pub struct GstVideoConverter {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    dmabuf_allocator: Option<DmaBufAllocator>,
    backend: GpuBackend,
}

impl GstVideoConverter {
    /// Create a new video converter with the specified GPU backend
    pub fn new(
        width: u32,
        height: u32,
        format: spa::param::video::VideoFormat,
        backend: GpuBackend,
    ) -> Result<Self, GstError> {
        // Convert PipeWire format to GStreamer format string
        let format_str = match format {
            spa::param::video::VideoFormat::RGB => "RGB",
            spa::param::video::VideoFormat::RGBA => "RGBA",
            spa::param::video::VideoFormat::RGBx => "RGBx",
            spa::param::video::VideoFormat::BGRx => "BGRx",
            spa::param::video::VideoFormat::YUY2 => "YUY2",
            spa::param::video::VideoFormat::I420 => "I420",
            _ => {
                return Err(GstError::InitError(format!(
                    "Unsupported format: {:?}",
                    format
                )))
            }
        };

        // Create pipeline elements
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

        // Configure appsrc
        let input_caps = gst::Caps::builder("video/x-raw")
            .field("format", format_str)
            .field("width", width as i32)
            .field("height", height as i32)
            .build();

        appsrc.set_caps(Some(&input_caps));
        appsrc.set_format(gst::Format::Time);
        appsrc.set_property("block", true);

        // Configure appsink to output RGBA
        let output_caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", width as i32)
            .field("height", height as i32)
            .build();

        appsink.set_caps(Some(&output_caps));
        appsink.set_property("emit-signals", false);
        appsink.set_property("sync", false);

        // Build the conversion pipeline based on selected backend
        let actual_backend = Self::build_pipeline(
            &pipeline,
            &appsrc,
            &appsink,
            &backend,
        )?;

        if actual_backend != backend {
            warn!(
                requested = %backend,
                actual = %actual_backend,
                "Requested backend unavailable, using fallback"
            );
        }

        info!(backend = %actual_backend, "Video converter initialized");

        // Set pipeline to playing state
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| GstError::PipelineError(format!("Failed to set pipeline to playing: {:?}", e)))?;

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
            dmabuf_allocator: None,
            backend: actual_backend,
        })
    }

    /// Build the GStreamer pipeline with the specified backend
    fn build_pipeline(
        pipeline: &gst::Pipeline,
        appsrc: &gst_app::AppSrc,
        appsink: &gst_app::AppSink,
        backend: &GpuBackend,
    ) -> Result<GpuBackend, GstError> {
        match backend {
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
            GpuBackend::Vaapi(device) => {
                match Self::try_build_vaapi_pipeline(pipeline, appsrc, appsink, &device.path) {
                    Ok(()) => Ok(backend.clone()),
                    Err(e) => {
                        warn!(error = %e, "Failed to create VA-API pipeline, falling back to CPU");
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

    /// Try to build a CUDA-accelerated pipeline
    fn try_build_cuda_pipeline(
        pipeline: &gst::Pipeline,
        appsrc: &gst_app::AppSrc,
        appsink: &gst_app::AppSink,
        device_id: i32,
    ) -> Result<(), GstError> {
        // CUDA pipeline: appsrc → cudaupload → cudaconvert → cudadownload → appsink
        let cudaupload = gst::ElementFactory::make("cudaupload")
            .property("cuda-device-id", device_id)
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create cudaupload: {}", e)))?;

        let cudaconvert = gst::ElementFactory::make("cudaconvert")
            .property("cuda-device-id", device_id)
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create cudaconvert: {}", e)))?;

        let cudadownload = gst::ElementFactory::make("cudadownload")
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create cudadownload: {}", e)))?;

        pipeline
            .add_many([
                appsrc.upcast_ref(),
                &cudaupload,
                &cudaconvert,
                &cudadownload,
                appsink.upcast_ref(),
            ])
            .map_err(|e| GstError::PipelineError(format!("Failed to add CUDA elements: {}", e)))?;

        gst::Element::link_many([
            appsrc.upcast_ref(),
            &cudaupload,
            &cudaconvert,
            &cudadownload,
            appsink.upcast_ref(),
        ])
        .map_err(|e| GstError::PipelineError(format!("Failed to link CUDA elements: {}", e)))?;

        info!(device_id, "CUDA pipeline created successfully");
        Ok(())
    }

    /// Try to build an OpenGL accelerated pipeline
    fn try_build_vaapi_pipeline(
        pipeline: &gst::Pipeline,
        appsrc: &gst_app::AppSrc,
        appsink: &gst_app::AppSink,
        _device_path: &str,
    ) -> Result<(), GstError> {
        // OpenGL pipeline: appsrc → glupload → glcolorconvert → gldownload → appsink
        let glupload = gst::ElementFactory::make("glupload")
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create glupload: {}", e)))?;

        let glcolorconvert = gst::ElementFactory::make("glcolorconvert")
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create glcolorconvert: {}", e)))?;

        let gldownload = gst::ElementFactory::make("gldownload")
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create gldownload: {}", e)))?;

        pipeline
            .add_many([
                appsrc.upcast_ref(),
                &glupload,
                &glcolorconvert,
                &gldownload,
                appsink.upcast_ref(),
            ])
            .map_err(|e| GstError::PipelineError(format!("Failed to add OpenGL elements: {}", e)))?;

        gst::Element::link_many([
            appsrc.upcast_ref(),
            &glupload,
            &glcolorconvert,
            &gldownload,
            appsink.upcast_ref(),
        ])
        .map_err(|e| GstError::PipelineError(format!("Failed to link OpenGL elements: {}", e)))?;

        info!("OpenGL pipeline created successfully");
        Ok(())
    }

    /// Build CPU (software) pipeline
    fn build_cpu_pipeline(
        pipeline: &gst::Pipeline,
        appsrc: &gst_app::AppSrc,
        appsink: &gst_app::AppSink,
    ) -> Result<(), GstError> {
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create videoconvert: {}", e)))?;

        pipeline
            .add_many([appsrc.upcast_ref(), &videoconvert, appsink.upcast_ref()])
            .map_err(|e| GstError::PipelineError(format!("Failed to add elements: {}", e)))?;

        gst::Element::link_many([appsrc.upcast_ref(), &videoconvert, appsink.upcast_ref()])
            .map_err(|e| GstError::PipelineError(format!("Failed to link elements: {}", e)))?;

        info!("CPU (software) pipeline created");
        Ok(())
    }

    /// Get the actual backend being used
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
        // Get or create cached DMA-BUF allocator
        let allocator = match &self.dmabuf_allocator {
            Some(alloc) => alloc.clone(),
            None => {
                let alloc = DmaBufAllocator::new();
                self.dmabuf_allocator = Some(alloc.clone());
                alloc
            }
        };

        // Wrap the DMA-BUF fd in GStreamer memory (ZERO COPY - no data copying!)
        // Use DONT_CLOSE flag because PipeWire owns the fd and will close it
        let memory = unsafe { allocator.alloc_with_flags(fd, size, FdMemoryFlags::DONT_CLOSE) }
            .map_err(|e| {
                GstError::ConversionError(format!("Failed to allocate DMA-BUF memory: {}", e))
            })?;

        // Create buffer and attach the DMA-BUF memory
        let mut buffer = gst::Buffer::new();
        {
            let buffer_ref = buffer.get_mut().unwrap();
            buffer_ref.append_memory(memory);
        }

        // Push to pipeline - GStreamer will handle the rest
        self.appsrc
            .push_buffer(buffer)
            .map_err(|e| GstError::ConversionError(format!("Failed to push DMA-BUF buffer: {:?}", e)))?;

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
