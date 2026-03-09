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

/// Registry holding all active lighting backends. Routes per-light operations
/// to the correct backend based on `BackendData`.
pub struct LightingRegistry {
    wiz: WizBackend,
    // Future backends: ha: Option<HomeAssistantBackend>,
}

impl LightingRegistry {
    pub fn new() -> Self {
        Self {
            wiz: WizBackend::new(),
        }
    }

    /// Discover lights from all backends concurrently.
    pub fn discover(&self) -> Pin<Box<dyn Future<Output = Vec<LightInfo>> + Send>> {
        let wiz_fut = self.wiz.discover();
        Box::pin(async move {
            let wiz_lights = wiz_fut.await;
            // Future: merge results from other backends
            wiz_lights
        })
    }

    /// Map a sampled color to a light color command, routed by backend.
    pub fn map_color(
        &self,
        data: &BackendData,
        r: u8,
        g: u8,
        b: u8,
        min_brightness: u8,
        white_temp: u16,
    ) -> LightColor {
        match data {
            BackendData::Wiz { .. } => self.wiz.map_color(r, g, b, min_brightness, white_temp),
        }
    }

    /// Partition targets by backend and dispatch each group concurrently.
    pub fn dispatch_colors(
        &self,
        targets: Vec<(LightInfo, LightColor)>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        // Partition by backend (currently only WiZ)
        let mut wiz_targets = Vec::new();
        for target in targets {
            match &target.0.backend_data {
                BackendData::Wiz { .. } => wiz_targets.push(target),
            }
        }

        let wiz_fut = self.wiz.dispatch_colors(wiz_targets);
        Box::pin(async move {
            wiz_fut.await;
            // Future: join with other backend futures
        })
    }

    /// Partition lights by backend, save each group, merge results.
    pub fn save_states(
        &self,
        lights: Vec<LightInfo>,
    ) -> Pin<Box<dyn Future<Output = Vec<SavedLightState>> + Send>> {
        let mut wiz_lights = Vec::new();
        for light in lights {
            match &light.backend_data {
                BackendData::Wiz { .. } => wiz_lights.push(light),
            }
        }

        let wiz_fut = self.wiz.save_states(wiz_lights);
        Box::pin(async move {
            let all = wiz_fut.await;
            // Future: merge results from other backends
            all
        })
    }

    /// Partition states by backend and restore each group concurrently.
    pub fn restore_states(
        &self,
        states: Vec<SavedLightState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let mut wiz_states = Vec::new();
        for state in states {
            match &state.info.backend_data {
                BackendData::Wiz { .. } => wiz_states.push(state),
            }
        }

        let wiz_fut = self.wiz.restore_states(wiz_states);
        Box::pin(async move {
            wiz_fut.await;
            // Future: join with other backend futures
        })
    }

    /// Format a short display ID, routed by the light's backend data.
    #[allow(dead_code)]
    pub fn short_id(&self, info: &LightInfo) -> String {
        match &info.backend_data {
            BackendData::Wiz { .. } => self.wiz.short_id(&info.id),
        }
    }

    /// Convenience: format a short display ID from just a `LightId`.
    /// Delegates to the appropriate backend (currently always WiZ).
    pub fn short_id_by_id(&self, id: &LightId) -> String {
        // When multiple backends exist, this could check ID format or try each.
        self.wiz.short_id(id)
    }
}

/// Build light color targets from pre-computed `sampled_color` on each region.
pub fn build_light_targets(
    regions: &[crate::region::Region],
    lights: &[LightInfo],
    registry: &LightingRegistry,
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
        let color = registry.map_color(&light.backend_data, r, g, b, min_brightness, white_temp);
        targets.push((light.clone(), color));
    }

    if targets.is_empty() {
        None
    } else {
        Some(targets)
    }
}
