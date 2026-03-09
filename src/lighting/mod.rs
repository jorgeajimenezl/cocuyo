pub mod wiz;

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use self::wiz::WizBackend;

/// Opaque light identifier. For WiZ this is the MAC address; for other
/// backends it may be an entity ID or similar unique key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LightId(pub String);

impl std::fmt::Display for LightId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Information about a discovered light.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightInfo {
    pub id: LightId,
    pub name: Option<String>,
    pub backend_data: BackendData,
}

/// Backend-specific data stored alongside each light.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BackendData {
    Wiz { ip: IpAddr, mac: String },
}

/// A color command ready to send to a light.
#[derive(Debug, Clone)]
pub struct LightColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub brightness: u8,
    pub color_temp: Option<u16>,
}

/// Saved state for restoring a light to its previous configuration.
#[derive(Debug, Clone)]
pub struct SavedLightState {
    pub info: LightInfo,
    pub backend_state: SavedBackendState,
}

/// Backend-specific saved state data.
#[derive(Debug, Clone)]
pub enum SavedBackendState {
    Wiz {
        was_on: bool,
        color: Option<(u8, u8, u8)>,
        brightness: Option<u8>,
        scene_id: Option<u16>,
        temperature: Option<u16>,
    },
}

/// Trait that all lighting backends must implement.
#[allow(dead_code)]
pub trait LightingOps: Send + Sync {
    fn discover(&self) -> Pin<Box<dyn Future<Output = Vec<LightInfo>> + Send>>;
    fn map_color(&self, r: u8, g: u8, b: u8, min_brightness: u8, white_temp: u16) -> LightColor;
    fn dispatch_colors(
        &self,
        targets: Vec<(LightInfo, LightColor)>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;
    fn save_states(
        &self,
        lights: Vec<LightInfo>,
    ) -> Pin<Box<dyn Future<Output = Vec<SavedLightState>> + Send>>;
    fn restore_states(
        &self,
        states: Vec<SavedLightState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;
    fn display_name(&self) -> &str;
    fn light_noun(&self) -> &str;
    fn short_id(&self, id: &LightId) -> String;
}

/// Top-level enum wrapping concrete lighting backends.
pub enum LightingBackend {
    Wiz(WizBackend),
}

impl LightingBackend {
    pub fn discover(&self) -> Pin<Box<dyn Future<Output = Vec<LightInfo>> + Send>> {
        match self {
            Self::Wiz(b) => b.discover(),
        }
    }

    pub fn map_color(
        &self,
        r: u8,
        g: u8,
        b: u8,
        min_brightness: u8,
        white_temp: u16,
    ) -> LightColor {
        match self {
            Self::Wiz(backend) => backend.map_color(r, g, b, min_brightness, white_temp),
        }
    }

    pub fn dispatch_colors(
        &self,
        targets: Vec<(LightInfo, LightColor)>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        match self {
            Self::Wiz(b) => b.dispatch_colors(targets),
        }
    }

    pub fn save_states(
        &self,
        lights: Vec<LightInfo>,
    ) -> Pin<Box<dyn Future<Output = Vec<SavedLightState>> + Send>> {
        match self {
            Self::Wiz(b) => b.save_states(lights),
        }
    }

    pub fn restore_states(
        &self,
        states: Vec<SavedLightState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        match self {
            Self::Wiz(b) => b.restore_states(states),
        }
    }

    pub fn short_id(&self, id: &LightId) -> String {
        match self {
            Self::Wiz(b) => b.short_id(id),
        }
    }
}

/// Build light color targets from pre-computed `sampled_color` on each region.
pub fn build_light_targets(
    regions: &[crate::region::Region],
    lights: &[LightInfo],
    backend: &LightingBackend,
    min_brightness: u8,
    white_temp: u16,
) -> Option<Vec<(LightInfo, LightColor)>> {
    let mut targets = Vec::new();

    for region in regions {
        let id = &region.light_id;
        let Some((r, g, b)) = region.sampled_color else {
            continue;
        };
        let Some(light) = lights.iter().find(|l| &l.id == id) else {
            continue;
        };
        let color = backend.map_color(r, g, b, min_brightness, white_temp);
        targets.push((light.clone(), color));
    }

    if targets.is_empty() {
        None
    } else {
        Some(targets)
    }
}
