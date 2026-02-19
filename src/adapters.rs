use tracing::info;

/// The value type used by the iced pick_list in the Settings UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterSelection {
    /// Let wgpu pick the default adapter.
    Auto,
    /// A specific adapter chosen by name.
    Named(String),
}

impl std::fmt::Display for AdapterSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "Auto (wgpu default)"),
            Self::Named(name) => write!(f, "{}", name),
        }
    }
}

/// Enumerates Vulkan adapters using a temporary wgpu Instance.
/// Called once before `iced::daemon().run()`.
pub fn enumerate_vulkan_adapters() -> Vec<String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });

    let adapters: Vec<String> = instance
        .enumerate_adapters(wgpu::Backends::VULKAN)
        .into_iter()
        .map(|adapter| adapter.get_info().name)
        .collect();

    info!(count = adapters.len(), "Enumerated Vulkan adapters");
    for a in &adapters {
        info!(adapter = %a, "Found adapter");
    }

    adapters
}

/// Builds the complete pick_list option list: [Auto, Named(a0), Named(a1), ...].
pub fn build_picker_options(adapters: &[String]) -> Vec<AdapterSelection> {
    let mut options = Vec::with_capacity(adapters.len() + 1);
    options.push(AdapterSelection::Auto);
    options.extend(adapters.iter().cloned().map(AdapterSelection::Named));
    options
}

/// Given a saved preference string, find the matching AdapterSelection.
/// Uses case-insensitive substring match, consistent with WGPU_ADAPTER_NAME.
pub fn resolve_selection(
    preferred: Option<&str>,
    adapters: &[String],
) -> AdapterSelection {
    let Some(pref) = preferred else {
        return AdapterSelection::Auto;
    };
    let pref_lower = pref.to_lowercase();
    adapters
        .iter()
        .find(|a| a.to_lowercase().contains(&pref_lower))
        .cloned()
        .map(AdapterSelection::Named)
        .unwrap_or(AdapterSelection::Auto)
}
