use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

use crate::{adapters::GpuAdapter, ambient::BulbInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub preferred_adapter: Option<GpuAdapter>,
    pub preferred_backend: Option<String>,
    #[serde(default)]
    pub saved_bulbs: Vec<BulbInfo>,
    #[serde(default)]
    pub selected_bulb_macs: Vec<String>,
    #[serde(default)]
    pub force_cpu_sampling: bool,
    #[serde(default = "default_bulb_update_ms")]
    pub bulb_update_interval_ms: u64,
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
    #[serde(default = "default_smooth_transitions")]
    pub smooth_transitions: bool,
}

fn default_minimize_to_tray() -> bool {
    true
}

fn default_capture_fps_limit() -> u32 {
    0
}

fn default_bulb_update_ms() -> u64 {
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

fn default_smooth_transitions() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            preferred_adapter: None,
            preferred_backend: None,
            saved_bulbs: Vec::new(),
            selected_bulb_macs: Vec::new(),
            force_cpu_sampling: false,
            bulb_update_interval_ms: default_bulb_update_ms(),
            min_brightness_percent: default_min_brightness(),
            white_color_temp: default_white_temp(),
            minimize_to_tray: default_minimize_to_tray(),
            capture_fps_limit: default_capture_fps_limit(),
            show_perf_overlay: false,
            capture_resolution_scale: default_capture_resolution_scale(),
            smooth_transitions: default_smooth_transitions(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_deserialize_round_trip() {
        let original = AppConfig::default();
        let toml_str = toml::to_string(&original).expect("serialize");
        let restored: AppConfig = toml::from_str(&toml_str).expect("deserialize");

        assert_eq!(restored.bulb_update_interval_ms, original.bulb_update_interval_ms);
        assert_eq!(restored.min_brightness_percent, original.min_brightness_percent);
        assert_eq!(restored.white_color_temp, original.white_color_temp);
        assert_eq!(restored.minimize_to_tray, original.minimize_to_tray);
        assert_eq!(restored.capture_fps_limit, original.capture_fps_limit);
        assert_eq!(restored.capture_resolution_scale, original.capture_resolution_scale);
        assert_eq!(restored.force_cpu_sampling, original.force_cpu_sampling);
        assert_eq!(restored.show_perf_overlay, original.show_perf_overlay);
        assert_eq!(restored.preferred_adapter, original.preferred_adapter);
        assert_eq!(restored.preferred_backend, original.preferred_backend);
        assert_eq!(restored.smooth_transitions, original.smooth_transitions);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        // Simulates an old config file that only has one field
        let toml_str = r#"force_cpu_sampling = true"#;
        let config: AppConfig = toml::from_str(toml_str).expect("partial parse");

        assert!(config.force_cpu_sampling);
        // All other fields should have their defaults
        assert_eq!(config.bulb_update_interval_ms, 150);
        assert_eq!(config.min_brightness_percent, 10);
        assert_eq!(config.white_color_temp, 6500);
        assert_eq!(config.capture_resolution_scale, 100);
        assert!(config.minimize_to_tray);
        assert!(config.smooth_transitions);
    }

    #[test]
    fn saved_bulbs_round_trip() {
        let mut config = AppConfig::default();
        config.saved_bulbs.push(BulbInfo {
            mac: "AA:BB:CC:DD:EE:FF".to_string(),
            ip: "192.168.1.42".parse().unwrap(),
            name: Some("Living Room".to_string()),
        });
        config.selected_bulb_macs.push("AA:BB:CC:DD:EE:FF".to_string());

        let toml_str = toml::to_string(&config).expect("serialize");
        let restored: AppConfig = toml::from_str(&toml_str).expect("deserialize");

        assert_eq!(restored.saved_bulbs.len(), 1);
        assert_eq!(restored.saved_bulbs[0].mac, "AA:BB:CC:DD:EE:FF");
        assert_eq!(restored.saved_bulbs[0].ip.to_string(), "192.168.1.42");
        assert_eq!(restored.saved_bulbs[0].name.as_deref(), Some("Living Room"));
        assert_eq!(restored.selected_bulb_macs, vec!["AA:BB:CC:DD:EE:FF"]);
    }
}
