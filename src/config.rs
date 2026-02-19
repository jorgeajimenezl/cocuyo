use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub preferred_adapter: Option<String>,
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
