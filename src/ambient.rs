use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq)]
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
pub fn map_to_bulb_color(
    r: u8,
    g: u8,
    b: u8,
    min_brightness: u8,
    white_temp: u16,
) -> (BulbColor, u8) {
    let max = r.max(g).max(b);
    if max == 0 {
        // Screen pixel is black — send dimmest white
        return (BulbColor::White(white_temp), min_brightness);
    }

    let brightness = ((max as u16 * 100) / 255).clamp(min_brightness as u16, 100) as u8;
    let scale = 255.0 / max as f64;
    let nr = (r as f64 * scale).round().min(255.0) as u8;
    let ng = (g as f64 * scale).round().min(255.0) as u8;
    let nb = (b as f64 * scale).round().min(255.0) as u8;

    if nr == 255 && ng == 255 && nb == 255 {
        (BulbColor::White(white_temp), brightness)
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

/// Build bulb color targets from pre-computed `sampled_color` on each region.
/// This avoids re-sampling the frame when colors have already been computed
/// (e.g. via GPU sampling).
pub fn build_bulb_targets(
    regions: &[crate::region::Region],
    bulbs: &[BulbInfo],
    min_brightness: u8,
    white_temp: u16,
) -> Option<Vec<(IpAddr, BulbColor, u8)>> {
    let mut targets = Vec::new();

    for region in regions {
        let mac = &region.bulb_mac;
        let Some((r, g, b)) = region.sampled_color else {
            continue;
        };
        let Some(bulb) = bulbs.iter().find(|b| &b.mac == mac) else {
            continue;
        };
        let (color, brightness) = map_to_bulb_color(r, g, b, min_brightness, white_temp);
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
            if let Some(b) = state.brightness
                && let Some(br) = wiz_lights_rs::Brightness::create(b)
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::region::Region;
    use crate::sampling::BoxedStrategy;

    fn make_region(mac: &str, color: Option<(u8, u8, u8)>) -> Region {
        Region {
            id: 0,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            bulb_mac: mac.to_string(),
            sampled_color: color,
            strategy: BoxedStrategy::default(),
        }
    }

    fn make_bulb(mac: &str) -> BulbInfo {
        BulbInfo {
            mac: mac.to_string(),
            ip: "192.168.1.100".parse().unwrap(),
            name: None,
        }
    }

    // -- map_to_bulb_color --

    #[test]
    fn black_pixel_returns_white_min_brightness() {
        let (color, brightness) = map_to_bulb_color(0, 0, 0, 10, 6500);
        assert_eq!(color, BulbColor::White(6500));
        assert_eq!(brightness, 10);
    }

    #[test]
    fn pure_red_normalizes_to_255() {
        let (color, _brightness) = map_to_bulb_color(128, 0, 0, 10, 6500);
        assert_eq!(color, BulbColor::Rgb(255, 0, 0));
    }

    #[test]
    fn white_pixel_returns_white_temp() {
        let (color, brightness) = map_to_bulb_color(255, 255, 255, 10, 6500);
        assert_eq!(color, BulbColor::White(6500));
        assert_eq!(brightness, 100);
    }

    #[test]
    fn near_white_returns_white() {
        // All channels equal → normalizes to (255, 255, 255) → White
        let (color, _) = map_to_bulb_color(200, 200, 200, 10, 4200);
        assert_eq!(color, BulbColor::White(4200));
    }

    #[test]
    fn mixed_color_preserves_ratios() {
        // (100, 50, 25) → max=100, scale=2.55 → (255, 128, 64) approximately
        let (color, _) = map_to_bulb_color(100, 50, 25, 10, 6500);
        match color {
            BulbColor::Rgb(r, g, b) => {
                assert_eq!(r, 255);
                // Check ratios are approximately maintained (50/100 ≈ g/255)
                assert!((g as f32 / 255.0 - 0.5).abs() < 0.02, "g ratio: {g}");
                assert!((b as f32 / 255.0 - 0.25).abs() < 0.02, "b ratio: {b}");
            }
            other => panic!("expected Rgb, got {other:?}"),
        }
    }

    #[test]
    fn brightness_clamps_to_min() {
        // Very dim pixel: max channel = 5 → brightness = 5*100/255 ≈ 1 → clamped to min (20)
        let (_, brightness) = map_to_bulb_color(5, 2, 1, 20, 6500);
        assert_eq!(brightness, 20);
    }

    // -- build_bulb_targets --

    #[test]
    fn empty_regions_returns_none() {
        let result = build_bulb_targets(&[], &[make_bulb("AA:BB")], 10, 6500);
        assert!(result.is_none());
    }

    #[test]
    fn matching_region_produces_target() {
        let regions = [make_region("AA:BB", Some((255, 0, 0)))];
        let bulbs = [make_bulb("AA:BB")];
        let targets = build_bulb_targets(&regions, &bulbs, 10, 6500).expect("should have targets");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "192.168.1.100".parse::<IpAddr>().unwrap());
    }
}
