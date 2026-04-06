use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

use crate::{adapters::GpuAdapter, ambient::BulbInfo};
use crate::region::Region;
use crate::sampling::BoxedStrategy;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRegion {
    /// Normalized 0.0-1.0 coordinates (fraction of frame dimensions)
    pub nx: f32,
    pub ny: f32,
    pub nw: f32,
    pub nh: f32,
    pub bulb_mac: String,
    pub strategy_id: String,
}

impl ProfileRegion {
    pub fn from_region(r: &Region, frame_w: f32, frame_h: f32) -> Self {
        Self {
            nx: r.x / frame_w,
            ny: r.y / frame_h,
            nw: r.width / frame_w,
            nh: r.height / frame_h,
            bulb_mac: r.bulb_mac.clone(),
            strategy_id: r.strategy.id().to_string(),
        }
    }

    pub fn to_region(&self, id: usize, frame_w: f32, frame_h: f32) -> Region {
        Region {
            id,
            x: self.nx * frame_w,
            y: self.ny * frame_h,
            width: self.nw * frame_w,
            height: self.nh * frame_h,
            bulb_mac: self.bulb_mac.clone(),
            sampled_color: None,
            strategy: BoxedStrategy::from_id(&self.strategy_id).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub regions: Vec<ProfileRegion>,
    pub selected_bulb_macs: Vec<String>,
    pub bulb_update_interval_ms: u64,
    pub min_brightness_percent: u8,
    pub white_color_temp: u16,
}

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
    #[serde(default)]
    pub profiles: Vec<Profile>,
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
            profiles: Vec::new(),
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

    #[test]
    fn profile_round_trip() {
        let mut config = AppConfig::default();
        config.profiles.push(Profile {
            name: "Gaming".to_string(),
            regions: vec![
                ProfileRegion {
                    nx: 0.0,
                    ny: 0.0,
                    nw: 0.5,
                    nh: 1.0,
                    bulb_mac: "AA:BB:CC:DD:EE:FF".to_string(),
                    strategy_id: "average".to_string(),
                },
                ProfileRegion {
                    nx: 0.5,
                    ny: 0.0,
                    nw: 0.5,
                    nh: 1.0,
                    bulb_mac: "11:22:33:44:55:66".to_string(),
                    strategy_id: "palette".to_string(),
                },
            ],
            selected_bulb_macs: vec![
                "AA:BB:CC:DD:EE:FF".to_string(),
                "11:22:33:44:55:66".to_string(),
            ],
            bulb_update_interval_ms: 100,
            min_brightness_percent: 15,
            white_color_temp: 6500,
        });

        let toml_str = toml::to_string_pretty(&config).expect("serialize");
        let restored: AppConfig = toml::from_str(&toml_str).expect("deserialize");

        assert_eq!(restored.profiles.len(), 1);
        let p = &restored.profiles[0];
        assert_eq!(p.name, "Gaming");
        assert_eq!(p.regions.len(), 2);
        assert_eq!(p.regions[0].strategy_id, "average");
        assert_eq!(p.regions[1].bulb_mac, "11:22:33:44:55:66");
        assert_eq!(p.bulb_update_interval_ms, 100);
        assert_eq!(p.min_brightness_percent, 15);
    }

    #[test]
    fn profile_region_normalization_round_trip() {
        let region = Region {
            id: 1,
            x: 480.0,
            y: 270.0,
            width: 960.0,
            height: 540.0,
            bulb_mac: "AA:BB:CC:DD:EE:FF".to_string(),
            sampled_color: Some((255, 0, 0)),
            strategy: BoxedStrategy::default(),
        };

        let pr = ProfileRegion::from_region(&region, 1920.0, 1080.0);
        assert!((pr.nx - 0.25).abs() < 1e-5);
        assert!((pr.ny - 0.25).abs() < 1e-5);
        assert!((pr.nw - 0.5).abs() < 1e-5);
        assert!((pr.nh - 0.5).abs() < 1e-5);

        let restored = pr.to_region(42, 1920.0, 1080.0);
        assert_eq!(restored.id, 42);
        assert!((restored.x - 480.0).abs() < 1.0);
        assert!((restored.y - 270.0).abs() < 1.0);
        assert!((restored.width - 960.0).abs() < 1.0);
        assert!((restored.height - 540.0).abs() < 1.0);
        assert!(restored.sampled_color.is_none());
    }
}
