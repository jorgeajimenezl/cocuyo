use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::frame::FrameData;

#[derive(Debug, Clone)]
pub struct BulbInfo {
    pub mac: String,
    pub ip: IpAddr,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SavedBulbState {
    pub ip: IpAddr,
    pub was_on: bool,
    pub color: Option<(u8, u8, u8)>,
    pub brightness: Option<u8>,
    pub scene_id: Option<u16>,
    pub temperature: Option<u16>,
}

/// A color command for a WiZ bulb. WiZ requires at least one RGB channel at 0xFF,
/// and all-white (255, 255, 255) must be sent as a color temperature instead.
#[derive(Debug, Clone)]
pub enum BulbColor {
    /// Saturated RGB with at least one channel at 255, never all three.
    Rgb(u8, u8, u8),
    /// White — use color temperature (Kelvin).
    White(u16),
}

/// Map a raw sampled pixel to a valid WiZ bulb color + brightness.
///
/// WiZ bulbs expect "pure" colors where the dominant channel is 0xFF.
/// The original intensity is extracted as brightness (10–100%).
/// Pure white (all channels equal) is converted to a 6500K temperature command.
pub fn map_to_bulb_color(r: u8, g: u8, b: u8) -> (BulbColor, u8) {
    let max = r.max(g).max(b);
    if max == 0 {
        // Screen pixel is black — send dimmest white
        return (BulbColor::White(6500), 10);
    }

    let brightness = ((max as u16 * 100) / 255).clamp(10, 100) as u8;
    let scale = 255.0 / max as f64;
    let nr = (r as f64 * scale).round().min(255.0) as u8;
    let ng = (g as f64 * scale).round().min(255.0) as u8;
    let nb = (b as f64 * scale).round().min(255.0) as u8;

    if nr == 255 && ng == 255 && nb == 255 {
        (BulbColor::White(6500), brightness)
    } else {
        (BulbColor::Rgb(nr, ng, nb), brightness)
    }
}

/// Send mapped colors to WiZ bulbs concurrently.
async fn send_colors_to_bulbs(targets: Vec<(IpAddr, BulbColor, u8)>) {
    let futs: Vec<_> = targets
        .into_iter()
        .map(|(ip, color, brightness)| async move {
            let ipv4 = match ip {
                IpAddr::V4(v4) => v4,
                IpAddr::V6(_) => {
                    tracing::warn!(%ip, "Skipping non-IPv4 bulb");
                    return;
                }
            };
            let light = wiz_lights_rs::Light::new(ipv4, None);
            let mut payload = wiz_lights_rs::Payload::new();
            match color {
                BulbColor::Rgb(r, g, b) => {
                    payload.color(&wiz_lights_rs::Color::rgb(r, g, b));
                }
                BulbColor::White(kelvin) => {
                    if let Some(k) = wiz_lights_rs::Kelvin::create(kelvin) {
                        payload.temp(&k);
                    }
                }
            }
            if let Some(br) = wiz_lights_rs::Brightness::create(brightness) {
                payload.brightness(&br);
            }
            if let Err(e) = light.set(&payload).await {
                tracing::warn!(error = %e, %ip, "Failed to send color to bulb");
            }
        })
        .collect();
    futures::future::join_all(futs).await;
}

/// Discover WiZ bulbs on the local network.
pub async fn discover_bulbs() -> Vec<BulbInfo> {
    match wiz_lights_rs::discover_bulbs(Duration::from_secs(5)).await {
        Ok(bulbs) => bulbs
            .into_iter()
            .map(|b| BulbInfo {
                ip: IpAddr::V4(b.ip),
                mac: b.mac.clone(),
                name: None,
            })
            .collect(),
        Err(e) => {
            tracing::error!(error = %e, "Bulb discovery failed");
            Vec::new()
        }
    }
}

/// Sample frame colors using region-based sampling.
/// Each region with a `bulb_mac` is sampled and mapped to the corresponding bulb.
pub fn sample_frame_for_regions(
    frame: &Arc<FrameData>,
    regions: &[crate::region::Region],
    bulbs: &[BulbInfo],
) -> Option<Vec<(IpAddr, BulbColor, u8)>> {
    let mut targets = Vec::new();

    for region in regions {
        let Some(mac) = &region.bulb_mac else { continue };
        let Some((r, g, b)) = frame.sample_region_average(
            region.x,
            region.y,
            region.width,
            region.height,
        ) else {
            continue;
        };
        let Some(bulb) = bulbs.iter().find(|b| &b.mac == mac) else {
            continue;
        };
        let (color, brightness) = map_to_bulb_color(r, g, b);
        targets.push((bulb.ip, color, brightness));
    }

    if targets.is_empty() {
        None
    } else {
        Some(targets)
    }
}

/// Dispatch sampled colors to bulbs. Returns a future suitable for `Task::perform`.
pub async fn dispatch_bulb_colors(targets: Vec<(IpAddr, BulbColor, u8)>) {
    send_colors_to_bulbs(targets).await;
}

/// Query each selected bulb's current state. Skips bulbs that fail.
pub async fn save_bulb_states(bulbs: Vec<BulbInfo>) -> Vec<SavedBulbState> {
    let futs: Vec<_> = bulbs
        .into_iter()
        .map(|bulb| async move {
            let ipv4 = match bulb.ip {
                IpAddr::V4(v4) => v4,
                IpAddr::V6(_) => return None,
            };
            let light = wiz_lights_rs::Light::new(ipv4, None);
            match light.get_status().await {
                Ok(status) => {
                    let color = status.color().map(|c| (c.red(), c.green(), c.blue()));
                    let brightness = status.brightness().map(|b| b.value());
                    let scene_id = status.scene().map(|s| s.id());
                    let temp = status.temp().map(|t| t.kelvin());
                    Some(SavedBulbState {
                        ip: bulb.ip,
                        was_on: status.emitting(),
                        color,
                        brightness,
                        scene_id,
                        temperature: temp,
                    })
                }
                Err(e) => {
                    tracing::warn!(error = %e, ip = %bulb.ip, "Failed to query bulb status");
                    None
                }
            }
        })
        .collect();

    let res: Vec<SavedBulbState> = futures::future::join_all(futs)
        .await
        .into_iter()
        .flatten()
        .collect();

    tracing::info!("Saved states for {} bulbs", res.len());
    res
}

/// Restore bulbs to their previously saved states. Best-effort.
///
/// The bulb's `getPilot` response includes both scene ID and the RGB values the
/// scene is currently producing. To avoid conflicts, we pick a single mode:
///   - If a scene was active, restore scene + brightness only.
///   - Otherwise restore color/cool/warm + brightness.
pub async fn restore_bulb_states(states: Vec<SavedBulbState>) {
    let futs: Vec<_> = states
        .into_iter()
        .map(|state| async move {
            let ipv4 = match state.ip {
                IpAddr::V4(v4) => v4,
                IpAddr::V6(_) => return,
            };
            let light = wiz_lights_rs::Light::new(ipv4, None);
            let mut payload = wiz_lights_rs::Payload::new();

            if let Some(id) = state.scene_id {
                // Scene mode: only send scene (+ brightness). The RGB values
                // from getPilot are just the scene's current output and would
                // conflict with the scene command.
                if let Some(scene) = wiz_lights_rs::SceneMode::create(id) {
                    payload.scene(&scene);
                }
            } else if let Some(k) = state.temperature {
                // Warm white mode
                if let Some(kv) = wiz_lights_rs::Kelvin::create(k) {
                    payload.temp(&kv);
                }
            } else if let Some((r, g, b)) = state.color {
                // RGB color mode
                payload.color(&wiz_lights_rs::Color::rgb(r, g, b));
            }

            // Brightness applies to all modes
            if let Some(b) = state.brightness &&
                let Some(br) = wiz_lights_rs::Brightness::create(b) {
                payload.brightness(&br);
            }

            if payload.is_valid() {
                if let Err(e) = light.set(&payload).await {
                    tracing::warn!(error = %e, ip = %state.ip, "Failed to restore bulb state");
                }
            }

            // If the bulb was off, turn it off after restoring settings
            if !state.was_on {
                if let Err(e) = light.set_power(&wiz_lights_rs::PowerMode::Off).await {
                    tracing::warn!(error = %e, ip = %state.ip, "Failed to turn off bulb");
                }
            }
        })
        .collect();
    futures::future::join_all(futs).await;
    tracing::info!("Bulb state restoration complete");
}
