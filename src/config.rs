use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

use crate::adapters::GpuAdapter;
use crate::lighting::LightInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub preferred_adapter: Option<GpuAdapter>,
    pub preferred_backend: Option<String>,
    #[serde(default, alias = "saved_bulbs")]
    pub saved_lights: Vec<LightInfo>,
    #[serde(default, alias = "selected_bulb_macs")]
    pub selected_light_ids: Vec<String>,
    #[serde(default)]
    pub force_cpu_sampling: bool,
    #[serde(default = "default_light_update_ms", alias = "bulb_update_interval_ms")]
    pub light_update_interval_ms: u64,
    #[serde(default = "default_min_brightness")]
    pub min_brightness_percent: u8,
    #[serde(default = "default_white_temp")]
    pub white_color_temp: u16,
    #[serde(default = "default_minimize_to_tray")]
    pub minimize_to_tray: bool,
    #[serde(default = "default_capture_fps_limit")]
    pub capture_fps_limit: u32,
    #[serde(default)]
    pub show_perf_overlay: bool,
    #[serde(default = "default_capture_resolution_scale")]
    pub capture_resolution_scale: u32,
}

fn default_minimize_to_tray() -> bool {
    true
}

fn default_capture_fps_limit() -> u32 {
    0
}

fn default_light_update_ms() -> u64 {
    150
}

fn default_min_brightness() -> u8 {
    10
}

fn default_white_temp() -> u16 {
    6500
}

fn default_capture_resolution_scale() -> u32 {
    100
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            preferred_adapter: None,
            preferred_backend: None,
            saved_lights: Vec::new(),
            selected_light_ids: Vec::new(),
            force_cpu_sampling: false,
            light_update_interval_ms: default_light_update_ms(),
            min_brightness_percent: default_min_brightness(),
            white_color_temp: default_white_temp(),
            minimize_to_tray: default_minimize_to_tray(),
            capture_fps_limit: default_capture_fps_limit(),
            show_perf_overlay: false,
            capture_resolution_scale: default_capture_resolution_scale(),
        }
    }
}

impl AppConfig {
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("cocuyo").join("config.toml"))
    }

    /// Loads config from disk. Returns Default on any error.
    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&content).unwrap_or_else(|e| {
            warn!("Failed to parse config: {}", e);
            Self::default()
        })
    }

    /// Persists config to disk. Creates parent directories if needed.
    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            warn!("Cannot resolve config directory");
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("Failed to create config dir: {}", e);
                return;
            }
        }
        match toml::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&path, content) {
                    warn!("Failed to write config: {}", e);
                }
            }
            Err(e) => warn!("Failed to serialize config: {}", e),
        }
    }
}
