#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuAdapterSelection {
    Auto,
    Named(String),
}

impl std::fmt::Display for GpuAdapterSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "Auto (wgpu default)"),
            Self::Named(name) => write!(f, "{}", name),
        }
    }
}

pub fn enumerate_adapters() -> Vec<String> {
    #[cfg(target_os = "linux")]
    let backends = wgpu::Backends::VULKAN;

    #[cfg(target_os = "windows")]
    let backends = wgpu::Backends::DX12 | wgpu::Backends::VULKAN;

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let backends = wgpu::Backends::PRIMARY;

    let adapters: Vec<String> = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..Default::default()
    })
    .enumerate_adapters(backends)
    .into_iter()
    .map(|adapter| adapter.get_info().name)
    .collect();

    adapters
}

pub fn build_picker_options(adapters: &[String]) -> Vec<GpuAdapterSelection> {
    let mut options = Vec::with_capacity(adapters.len() + 1);
    options.push(GpuAdapterSelection::Auto);
    options.extend(adapters.iter().cloned().map(GpuAdapterSelection::Named));
    options
}

/// Given a saved preference string, find the matching AdapterSelection.
/// Uses case-insensitive substring match, consistent with WGPU_ADAPTER_NAME.
pub fn resolve_selection(preferred: Option<&str>, adapters: &[String]) -> GpuAdapterSelection {
    let Some(pref) = preferred else {
        return GpuAdapterSelection::Auto;
    };
    let pref_lower = pref.to_lowercase();
    adapters
        .iter()
        .find(|a| a.to_lowercase().contains(&pref_lower))
        .cloned()
        .map(GpuAdapterSelection::Named)
        .unwrap_or(GpuAdapterSelection::Auto)
}
