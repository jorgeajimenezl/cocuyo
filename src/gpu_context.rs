use std::sync::OnceLock;

static GPU_CONTEXT: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();

/// Store the wgpu Device and Queue for use outside the shader widget.
/// Called once from `VideoPipeline::new()`.
pub fn set_gpu_context(device: wgpu::Device, queue: wgpu::Queue) {
    let _ = GPU_CONTEXT.set((device, queue));
}

/// Retrieve the stored wgpu Device and Queue, if available.
pub fn get_gpu_context() -> Option<&'static (wgpu::Device, wgpu::Queue)> {
    GPU_CONTEXT.get()
}
