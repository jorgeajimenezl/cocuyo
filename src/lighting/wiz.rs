use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::time::Duration;

use super::{
    BackendData, LightColor, LightId, LightInfo, LightingOps, SavedBackendState, SavedLightState,
};

pub struct WizBackend;

impl WizBackend {
    pub fn new() -> Self {
        Self
    }
}

/// A color command for a WiZ bulb. WiZ requires at least one RGB channel at 0xFF,
/// and all-white (255, 255, 255) must be sent as a color temperature instead.
#[derive(Debug, Clone)]
enum WizColor {
    /// Saturated RGB with at least one channel at 255, never all three.
    Rgb(u8, u8, u8),
    /// White -- use color temperature (Kelvin).
    White(u16),
}

impl LightingOps for WizBackend {
    fn discover(&self) -> Pin<Box<dyn Future<Output = Vec<LightInfo>> + Send>> {
        Box::pin(async {
            match wiz_lights_rs::discover_bulbs(Duration::from_secs(5)).await {
                Ok(bulbs) => bulbs
                    .into_iter()
                    .map(|b| LightInfo {
                        id: LightId(b.mac.clone()),
                        name: None,
                        backend_data: BackendData::Wiz {
                            ip: IpAddr::V4(b.ip),
                            mac: b.mac,
                        },
                    })
                    .collect(),
                Err(e) => {
                    tracing::error!(error = %e, "Bulb discovery failed");
                    Vec::new()
                }
            }
        })
    }

    fn map_color(&self, r: u8, g: u8, b: u8, min_brightness: u8, white_temp: u16) -> LightColor {
        let max = r.max(g).max(b);
        if max == 0 {
            return LightColor {
                r: 0,
                g: 0,
                b: 0,
                brightness: min_brightness,
                color_temp: Some(white_temp),
            };
        }

        let brightness = ((max as u16 * 100) / 255).clamp(min_brightness as u16, 100) as u8;
        let scale = 255.0 / max as f64;
        let nr = (r as f64 * scale).round().min(255.0) as u8;
        let ng = (g as f64 * scale).round().min(255.0) as u8;
        let nb = (b as f64 * scale).round().min(255.0) as u8;

        if nr == 255 && ng == 255 && nb == 255 {
            LightColor {
                r: nr,
                g: ng,
                b: nb,
                brightness,
                color_temp: Some(white_temp),
            }
        } else {
            LightColor {
                r: nr,
                g: ng,
                b: nb,
                brightness,
                color_temp: None,
            }
        }
    }

    fn dispatch_colors(
        &self,
        targets: Vec<(LightInfo, LightColor)>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let futs: Vec<_> = targets
                .into_iter()
                .map(|(info, color)| async move {
                    let BackendData::Wiz { ip, .. } = &info.backend_data;
                    let ipv4 = match ip {
                        IpAddr::V4(v4) => *v4,
                        IpAddr::V6(_) => {
                            tracing::warn!(%ip, "Skipping non-IPv4 bulb");
                            return;
                        }
                    };

                    let wiz_color = if let Some(temp) = color.color_temp {
                        WizColor::White(temp)
                    } else {
                        WizColor::Rgb(color.r, color.g, color.b)
                    };

                    let light = wiz_lights_rs::Light::new(ipv4, None);
                    let mut payload = wiz_lights_rs::Payload::new();
                    match wiz_color {
                        WizColor::Rgb(r, g, b) => {
                            payload.color(&wiz_lights_rs::Color::rgb(r, g, b));
                        }
                        WizColor::White(kelvin) => {
                            if let Some(k) = wiz_lights_rs::Kelvin::create(kelvin) {
                                payload.temp(&k);
                            }
                        }
                    }
                    if let Some(br) = wiz_lights_rs::Brightness::create(color.brightness) {
                        payload.brightness(&br);
                    }
                    if let Err(e) = light.set(&payload).await {
                        tracing::warn!(error = %e, %ip, "Failed to send color to bulb");
                    }
                })
                .collect();
            futures::future::join_all(futs).await;
        })
    }

    fn save_states(
        &self,
        lights: Vec<LightInfo>,
    ) -> Pin<Box<dyn Future<Output = Vec<SavedLightState>> + Send>> {
        Box::pin(async move {
            let futs: Vec<_> = lights
                .into_iter()
                .map(|info| async {
                    let BackendData::Wiz { ip, .. } = &info.backend_data;
                    let ipv4 = match ip {
                        IpAddr::V4(v4) => *v4,
                        IpAddr::V6(_) => return None,
                    };
                    let light = wiz_lights_rs::Light::new(ipv4, None);
                    match light.get_status().await {
                        Ok(status) => {
                            let color = status.color().map(|c| (c.red(), c.green(), c.blue()));
                            let brightness = status.brightness().map(|b| b.value());
                            let scene_id = status.scene().map(|s| s.id());
                            let temp = status.temp().map(|t| t.kelvin());
                            Some(SavedLightState {
                                info,
                                backend_state: SavedBackendState::Wiz {
                                    was_on: status.emitting(),
                                    color,
                                    brightness,
                                    scene_id,
                                    temperature: temp,
                                },
                            })
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, ip = %ip, "Failed to query bulb status");
                            None
                        }
                    }
                })
                .collect();

            let res: Vec<SavedLightState> = futures::future::join_all(futs)
                .await
                .into_iter()
                .flatten()
                .collect();

            tracing::info!("Saved states for {} lights", res.len());
            res
        })
    }

    fn restore_states(
        &self,
        states: Vec<SavedLightState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let futs: Vec<_> = states
                .into_iter()
                .map(|state| async move {
                    let BackendData::Wiz { ip, .. } = &state.info.backend_data;
                    let ipv4 = match ip {
                        IpAddr::V4(v4) => *v4,
                        IpAddr::V6(_) => return,
                    };

                    let SavedBackendState::Wiz {
                        was_on,
                        color,
                        brightness,
                        scene_id,
                        temperature,
                    } = &state.backend_state;

                    let light = wiz_lights_rs::Light::new(ipv4, None);
                    let mut payload = wiz_lights_rs::Payload::new();

                    if let Some(id) = scene_id {
                        if let Some(scene) = wiz_lights_rs::SceneMode::create(*id) {
                            payload.scene(&scene);
                        }
                    } else if let Some(k) = temperature {
                        if let Some(kv) = wiz_lights_rs::Kelvin::create(*k) {
                            payload.temp(&kv);
                        }
                    } else if let Some((r, g, b)) = color {
                        payload.color(&wiz_lights_rs::Color::rgb(*r, *g, *b));
                    }

                    if let Some(b) = brightness
                        && let Some(br) = wiz_lights_rs::Brightness::create(*b)
                    {
                        payload.brightness(&br);
                    }

                    if payload.is_valid() {
                        if let Err(e) = light.set(&payload).await {
                            tracing::warn!(error = %e, %ip, "Failed to restore bulb state");
                        }
                    }

                    if !was_on {
                        if let Err(e) = light.set_power(&wiz_lights_rs::PowerMode::Off).await {
                            tracing::warn!(error = %e, %ip, "Failed to turn off bulb");
                        }
                    }
                })
                .collect();
            futures::future::join_all(futs).await;
            tracing::info!("Light state restoration complete");
        })
    }

    fn display_name(&self) -> &str {
        "WiZ"
    }

    fn light_noun(&self) -> &str {
        "Bulb"
    }

    fn short_id(&self, id: &LightId) -> String {
        let s = &id.0;
        s[s.len().saturating_sub(8)..].to_string()
    }
}
