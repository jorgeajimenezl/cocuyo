use tracing::info;

/// Simplified adapter device type for display purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterDeviceType {
    Integrated,
    Discrete,
    Other,
}

/// Describes a single wgpu/Vulkan adapter available on the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuAdapterInfo {
    pub name: String,
    pub device_type: AdapterDeviceType,
}

impl std::fmt::Display for GpuAdapterInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.device_type {
            AdapterDeviceType::Integrated => write!(f, "{} (Integrated)", self.name),
            AdapterDeviceType::Discrete => write!(f, "{} (Discrete)", self.name),
            AdapterDeviceType::Other => write!(f, "{}", self.name),
        }
    }
}

/// The value type used by the iced pick_list in the Settings UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterSelection {
    /// Let wgpu pick the default adapter.
    Auto,
    /// A specific adapter chosen by name.
    Named(GpuAdapterInfo),
}

impl std::fmt::Display for AdapterSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "Auto (wgpu default)"),
            Self::Named(info) => write!(f, "{}", info),
        }
    }
}

/// Enumerates Vulkan adapters using a temporary wgpu Instance.
/// Called once before `iced::daemon().run()`.
pub fn enumerate_vulkan_adapters() -> Vec<GpuAdapterInfo> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });

    let adapters: Vec<GpuAdapterInfo> = instance
        .enumerate_adapters(wgpu::Backends::VULKAN)
        .into_iter()
        .map(|adapter| {
            let info = adapter.get_info();
            GpuAdapterInfo {
                name: info.name.clone(),
                device_type: match info.device_type {
                    wgpu::DeviceType::IntegratedGpu => AdapterDeviceType::Integrated,
                    wgpu::DeviceType::DiscreteGpu => AdapterDeviceType::Discrete,
                    _ => AdapterDeviceType::Other,
                },
            }
        })
        .collect();

    info!(count = adapters.len(), "Enumerated Vulkan adapters");
    for a in &adapters {
        info!(adapter = %a, "Found adapter");
    }

    adapters
}

/// Builds the complete pick_list option list: [Auto, Named(a0), Named(a1), ...].
pub fn build_picker_options(adapters: &[GpuAdapterInfo]) -> Vec<AdapterSelection> {
    let mut options = Vec::with_capacity(adapters.len() + 1);
    options.push(AdapterSelection::Auto);
    options.extend(adapters.iter().cloned().map(AdapterSelection::Named));
    options
}

/// Given a saved preference string, find the matching AdapterSelection.
/// Uses case-insensitive substring match, consistent with WGPU_ADAPTER_NAME.
pub fn resolve_selection(
    preferred: Option<&str>,
    adapters: &[GpuAdapterInfo],
) -> AdapterSelection {
    let Some(pref) = preferred else {
        return AdapterSelection::Auto;
    };
    let pref_lower = pref.to_lowercase();
    adapters
        .iter()
        .find(|a| a.name.to_lowercase().contains(&pref_lower))
        .cloned()
        .map(AdapterSelection::Named)
        .unwrap_or(AdapterSelection::Auto)
}
