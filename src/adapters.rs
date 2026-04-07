use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuAdapter {
    pub name: String,
    pub backend: wgpu::Backend,
}

impl std::fmt::Display for GpuAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.backend)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuAdapterSelection {
    Auto,
    Named(GpuAdapter),
}

impl std::fmt::Display for GpuAdapterSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "Auto (wgpu default)"),
            Self::Named(adapter) => write!(f, "{}", adapter),
        }
    }
}

pub fn enumerate_adapters() -> Vec<GpuAdapter> {
    #[cfg(target_os = "linux")]
    let backends = wgpu::Backends::VULKAN;

    #[cfg(target_os = "windows")]
    let backends = wgpu::Backends::DX12 | wgpu::Backends::VULKAN;

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let backends = wgpu::Backends::PRIMARY;

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    });
    let adapters: Vec<GpuAdapter> =
        futures::executor::block_on(instance.enumerate_adapters(backends))
            .into_iter()
            .map(|adapter| {
                let info = adapter.get_info();
                GpuAdapter {
                    name: info.name,
                    backend: info.backend,
                }
            })
            .collect();

    adapters
}

pub fn build_picker_options(adapters: &[GpuAdapter]) -> Vec<GpuAdapterSelection> {
    let mut options = Vec::with_capacity(adapters.len() + 1);
    options.push(GpuAdapterSelection::Auto);
    options.extend(adapters.iter().cloned().map(GpuAdapterSelection::Named));
    options
}

/// Given a saved preference string, find the matching AdapterSelection.
/// Uses case-insensitive substring match, consistent with WGPU_ADAPTER_NAME.
pub fn resolve_selection(
    preferred: Option<&GpuAdapter>,
    adapters: &[GpuAdapter],
) -> GpuAdapterSelection {
    let Some(pref) = preferred else {
        return GpuAdapterSelection::Auto;
    };
    let pref_lower = pref.name.to_lowercase();
    adapters
        .iter()
        .find(|a| a.name.to_lowercase().contains(&pref_lower) && a.backend == pref.backend)
        .cloned()
        .map(GpuAdapterSelection::Named)
        .unwrap_or(GpuAdapterSelection::Auto)
}
