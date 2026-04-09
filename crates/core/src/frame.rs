use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Boxed error type for GPU import failures.
pub type ImportError = Box<dyn std::error::Error + Send + Sync>;

/// Atomic flag for tracking whether a zero-copy import path is still viable.
/// Once import fails, the path is disabled until explicitly reset.
pub struct ImportGuard(AtomicBool);

impl ImportGuard {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn is_available(&self) -> bool {
        !self.0.load(Ordering::Relaxed)
    }

    pub fn mark_failed(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Trait implemented by each platform's zero-copy frame type.
///
/// Platform crates provide a concrete struct (e.g. `DmaBufFrame`,
/// `HeldFrame`, `IOSurfaceFrame`) and implement this trait. The binary and
/// `cocuyo-sampling` interact with frames exclusively through
/// [`FrameData`] — they never need to match on platform variants.
pub trait GpuFrame: Send + Sync + std::fmt::Debug {
    fn width(&self) -> u32;
    fn height(&self) -> u32;

    /// Import this frame as a wgpu texture, returning the native pixel format.
    ///
    /// On a permanent failure the implementation is responsible for disabling
    /// its own zero-copy path (e.g. via an `ImportGuard`) before returning the
    /// error, so callers don't need to know about platform-level state.
    fn import_gpu(
        &self,
        device: &wgpu::Device,
    ) -> Result<(wgpu::Texture, wgpu::TextureFormat), ImportError>;

    /// Read pixel data from this frame as tightly-packed BGRA bytes, for CPU
    /// sampling fallbacks. Returns `None` if readback is not possible.
    fn read_pixels_bgra(&self) -> Option<Vec<u8>>;
}

/// A captured video frame, either GPU-backed or already on the CPU.
#[derive(Debug)]
pub enum FrameData {
    /// A platform-specific zero-copy GPU frame (DMA-BUF, IOSurface, D3D shared texture).
    Gpu(Box<dyn GpuFrame>),
    /// Raw BGRA pixel data already in CPU memory.
    Cpu {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
}

impl FrameData {
    pub fn width(&self) -> u32 {
        match self {
            FrameData::Gpu(g) => g.width(),
            FrameData::Cpu { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            FrameData::Gpu(g) => g.height(),
            FrameData::Cpu { height, .. } => *height,
        }
    }

    /// Returns a reference to the BGRA pixel data for `Cpu` frames.
    /// GPU frames return `None` — use [`convert_to_cpu`] for readback.
    pub fn pixels(&self) -> Option<&[u8]> {
        match self {
            FrameData::Cpu { data, .. } => Some(data.as_slice()),
            FrameData::Gpu(_) => None,
        }
    }

    /// Convert this frame to a `Cpu` variant. For `Cpu` frames the Arc is
    /// cloned cheaply. For `Gpu` frames the platform impl reads back pixel data.
    pub fn convert_to_cpu(self: &Arc<Self>) -> Option<Arc<FrameData>> {
        match self.as_ref() {
            FrameData::Cpu { .. } => Some(self.clone()),
            FrameData::Gpu(g) => {
                let (width, height) = (g.width(), g.height());
                match g.read_pixels_bgra() {
                    Some(data) => Some(Arc::new(FrameData::Cpu { data, width, height })),
                    None => {
                        tracing::error!("Failed to read GPU frame pixels for CPU fallback");
                        None
                    }
                }
            }
        }
    }
}
