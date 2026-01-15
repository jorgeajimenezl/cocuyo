use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_allocators::{DmaBufAllocator, FdMemoryFlags};
use gstreamer_app as gst_app;
use pipewire::spa;
use std::os::fd::RawFd;

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

pub struct GstVideoConverter {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    dmabuf_allocator: Option<DmaBufAllocator>,
}

impl GstVideoConverter {
    pub fn new(
        width: u32,
        height: u32,
        format: spa::param::video::VideoFormat,
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

        let videoconvert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| GstError::PipelineError(format!("Failed to create videoconvert: {}", e)))?;

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

        // Add elements to pipeline
        pipeline
            .add_many([appsrc.upcast_ref(), &videoconvert, appsink.upcast_ref()])
            .map_err(|e| GstError::PipelineError(format!("Failed to add elements: {}", e)))?;

        // Link elements
        gst::Element::link_many([appsrc.upcast_ref(), &videoconvert, appsink.upcast_ref()])
            .map_err(|e| GstError::PipelineError(format!("Failed to link elements: {}", e)))?;

        // Set pipeline to playing state
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| GstError::PipelineError(format!("Failed to set pipeline to playing: {:?}", e)))?;

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
            dmabuf_allocator: None,
        })
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

    pub fn push_dmabuf(
        &mut self,
        fd: RawFd,
        size: usize,
    ) -> Result<(), GstError> {
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
        let memory = unsafe {
            allocator.alloc_with_flags(fd, size, FdMemoryFlags::DONT_CLOSE)
        }
        .map_err(|e| GstError::ConversionError(format!("Failed to allocate DMA-BUF memory: {}", e)))?;

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
