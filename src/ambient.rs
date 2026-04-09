use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

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

/// Per-channel threshold below which a color change is considered noise.
const COLOR_CHANGE_THRESHOLD: u8 = 4;

/// Returns `true` if the new bulb command differs meaningfully from the old one.
fn color_changed(old: &(BulbColor, u8), new: &(BulbColor, u8)) -> bool {
    let threshold = COLOR_CHANGE_THRESHOLD;
    if old.1.abs_diff(new.1) > threshold {
        return true;
    }
    match (&old.0, &new.0) {
        (BulbColor::Rgb(r1, g1, b1), BulbColor::Rgb(r2, g2, b2)) => {
            r1.abs_diff(*r2) > threshold
                || g1.abs_diff(*g2) > threshold
                || b1.abs_diff(*b2) > threshold
        }
        (BulbColor::White(t1), BulbColor::White(t2)) => t1 != t2,
        _ => true, // variant changed (e.g. Rgb ↔ White)
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
/// Filters out bulbs whose color hasn't meaningfully changed since the last dispatch.
///
/// Returns `None` if no bulbs need updating. On `Some`, returns the dispatch targets
/// and a list of `(mac, color, brightness)` entries the caller should insert into
/// `last_sent` after a successful dispatch.
pub fn build_bulb_targets(
    regions: &[cocuyo_sampling::Region],
    bulbs: &[BulbInfo],
    min_brightness: u8,
    white_temp: u16,
    last_sent: &HashMap<String, (BulbColor, u8)>,
) -> Option<(Vec<(IpAddr, BulbColor, u8)>, Vec<(String, BulbColor, u8)>)> {
    let mut targets = Vec::new();
    let mut new_entries = Vec::new();

    for region in regions {
        let mac = &region.bulb_mac;
        let Some((r, g, b)) = region.sampled_color else {
            continue;
        };
        let Some(bulb) = bulbs.iter().find(|b| &b.mac == mac) else {
            continue;
        };
        let (color, brightness) = map_to_bulb_color(r, g, b, min_brightness, white_temp);

        if let Some(prev) = last_sent.get(mac)
            && !color_changed(prev, &(color.clone(), brightness))
        {
            continue;
        }

        new_entries.push((mac.clone(), color.clone(), brightness));
        targets.push((bulb.ip, color, brightness));
    }

    if targets.is_empty() {
        None
    } else {
        Some((targets, new_entries))
    }
}

/// Dispatch sampled colors to bulbs. Returns a future suitable for `Task::perform`.
pub async fn dispatch_bulb_colors(targets: Vec<(IpAddr, BulbColor, u8)>) {
    send_colors_to_bulbs(targets).await;
}

/// Per-bulb exponential moving average smoother for sampled colors.
///
/// Interpolates between the current smoothed color and the new sample each
/// dispatch cycle, reducing flickering and jarring transitions on the bulbs.
/// The smoothing factor (alpha) controls responsiveness: lower = smoother but
/// slower to react, higher = more responsive but less smooth.
const SMOOTH_ALPHA: f32 = 0.35;

pub struct ColorSmoother {
    state: HashMap<String, (f32, f32, f32)>,
    last_update: Option<Instant>,
}

impl ColorSmoother {
    pub fn new() -> Self {
        Self {
            state: HashMap::new(),
            last_update: None,
        }
    }

    /// Apply smoothing to a region's sampled color. Returns the smoothed RGB.
    /// If this is the first sample for a bulb, snaps directly to the target.
    pub fn smooth(&mut self, mac: &str, target: (u8, u8, u8)) -> (u8, u8, u8) {
        let tf = (target.0 as f32, target.1 as f32, target.2 as f32);

        // Scale alpha by time since last update to keep smoothing consistent
        // regardless of update interval. At 150ms intervals alpha ≈ SMOOTH_ALPHA.
        let alpha = if let Some(last) = self.last_update {
            let dt = last.elapsed().as_secs_f32();
            // Normalize to 150ms reference interval
            (1.0 - (1.0 - SMOOTH_ALPHA).powf(dt / 0.15)).clamp(0.0, 1.0)
        } else {
            1.0 // first frame: snap
        };

        let smoothed = if let Some(cur) = self.state.get_mut(mac) {
            let next = (
                cur.0 + (tf.0 - cur.0) * alpha,
                cur.1 + (tf.1 - cur.1) * alpha,
                cur.2 + (tf.2 - cur.2) * alpha,
            );
            *cur = next;
            next
        } else {
            self.state.insert(mac.to_owned(), tf);
            tf // first sample for this bulb: snap
        };

        (
            smoothed.0.round().clamp(0.0, 255.0) as u8,
            smoothed.1.round().clamp(0.0, 255.0) as u8,
            smoothed.2.round().clamp(0.0, 255.0) as u8,
        )
    }

    /// Drop entries for bulbs no longer in use so state doesn't grow unbounded.
    pub fn retain<F: FnMut(&str) -> bool>(&mut self, mut keep: F) {
        self.state.retain(|k, _| keep(k));
    }

    /// Call after each dispatch cycle to update the timestamp.
    pub fn mark_updated(&mut self) {
        self.last_update = Some(Instant::now());
    }

    /// Clear all smoother state (e.g. when ambient stops).
    pub fn clear(&mut self) {
        self.state.clear();
        self.last_update = None;
    }
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

            if payload.is_valid()
                && let Err(e) = light.set(&payload).await
            {
                tracing::warn!(error = %e, ip = %state.ip, "Failed to restore bulb state");
            }

            // If the bulb was off, turn it off after restoring settings
            if !state.was_on
                && let Err(e) = light.set_power(&wiz_lights_rs::PowerMode::Off).await
            {
                tracing::warn!(error = %e, ip = %state.ip, "Failed to turn off bulb");
            }
        })
        .collect();
    futures::future::join_all(futs).await;
    tracing::info!("Bulb state restoration complete");
}

#[cfg(test)]
mod tests {
    use super::*;
    use cocuyo_sampling::BoxedStrategy;
    use cocuyo_sampling::Region;

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

    // -- color_changed --

    #[test]
    fn identical_rgb_is_not_changed() {
        let a = (BulbColor::Rgb(255, 100, 50), 80u8);
        assert!(!color_changed(&a, &a));
    }

    #[test]
    fn small_rgb_delta_is_not_changed() {
        let a = (BulbColor::Rgb(255, 100, 50), 80u8);
        let b = (BulbColor::Rgb(255, 103, 47), 82u8);
        assert!(!color_changed(&a, &b));
    }

    #[test]
    fn large_rgb_delta_is_changed() {
        let a = (BulbColor::Rgb(255, 100, 50), 80u8);
        let b = (BulbColor::Rgb(255, 100, 60), 80u8);
        assert!(color_changed(&a, &b));
    }

    #[test]
    fn brightness_delta_triggers_change() {
        let a = (BulbColor::Rgb(255, 100, 50), 80u8);
        let b = (BulbColor::Rgb(255, 100, 50), 90u8);
        assert!(color_changed(&a, &b));
    }

    #[test]
    fn variant_switch_is_changed() {
        let a = (BulbColor::Rgb(255, 255, 200), 80u8);
        let b = (BulbColor::White(6500), 80u8);
        assert!(color_changed(&a, &b));
    }

    #[test]
    fn identical_white_is_not_changed() {
        let a = (BulbColor::White(6500), 50u8);
        assert!(!color_changed(&a, &a));
    }

    #[test]
    fn white_temp_change_is_changed() {
        let a = (BulbColor::White(6500), 50u8);
        let b = (BulbColor::White(4200), 50u8);
        assert!(color_changed(&a, &b));
    }

    // -- build_bulb_targets --

    #[test]
    fn empty_regions_returns_none() {
        let result = build_bulb_targets(&[], &[make_bulb("AA:BB")], 10, 6500, &HashMap::new());
        assert!(result.is_none());
    }

    #[test]
    fn matching_region_produces_target() {
        let regions = [make_region("AA:BB", Some((255, 0, 0)))];
        let bulbs = [make_bulb("AA:BB")];
        let (targets, _) = build_bulb_targets(&regions, &bulbs, 10, 6500, &HashMap::new())
            .expect("should have targets");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "192.168.1.100".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn unchanged_color_is_filtered_out() {
        let regions = [make_region("AA:BB", Some((255, 0, 0)))];
        let bulbs = [make_bulb("AA:BB")];
        // First call to populate cache entries
        let (_, new_entries) =
            build_bulb_targets(&regions, &bulbs, 10, 6500, &HashMap::new()).unwrap();
        let mut cache: HashMap<String, (BulbColor, u8)> = HashMap::new();
        for (mac, color, brightness) in new_entries {
            cache.insert(mac, (color, brightness));
        }
        // Second call with same colors should return None (all filtered)
        let result = build_bulb_targets(&regions, &bulbs, 10, 6500, &cache);
        assert!(result.is_none());
    }

    #[test]
    fn changed_color_passes_through() {
        let bulbs = [make_bulb("AA:BB")];
        // Populate cache with red
        let mut cache: HashMap<String, (BulbColor, u8)> = HashMap::new();
        cache.insert("AA:BB".to_string(), (BulbColor::Rgb(255, 0, 0), 100));
        // Now region has green
        let regions = [make_region("AA:BB", Some((0, 255, 0)))];
        let result = build_bulb_targets(&regions, &bulbs, 10, 6500, &cache);
        assert!(result.is_some());
    }

    // -- ColorSmoother --

    #[test]
    fn smoother_first_sample_snaps_to_target() {
        let mut s = ColorSmoother::new();
        let result = s.smooth("AA:BB", (200, 100, 50));
        assert_eq!(result, (200, 100, 50));
    }

    #[test]
    fn smoother_interpolates_toward_target() {
        let mut s = ColorSmoother::new();
        s.smooth("AA:BB", (0, 0, 0));
        // Simulate 10ms elapsed by setting last_update in the past
        s.last_update = Some(Instant::now() - Duration::from_millis(10));
        let result = s.smooth("AA:BB", (255, 255, 255));
        // Should move toward white but not reach it
        assert!(result.0 > 0 && result.0 < 255, "r={}", result.0);
        assert!(result.1 > 0 && result.1 < 255, "g={}", result.1);
        assert!(result.2 > 0 && result.2 < 255, "b={}", result.2);
    }

    #[test]
    fn smoother_converges_over_many_steps() {
        let mut s = ColorSmoother::new();
        s.smooth("AA:BB", (0, 0, 0));
        // Simulate 50 updates at 150ms intervals (deterministic, no sleeping)
        for _ in 0..50 {
            s.last_update = Some(Instant::now() - Duration::from_millis(150));
            s.smooth("AA:BB", (255, 255, 255));
        }
        s.last_update = Some(Instant::now() - Duration::from_millis(150));
        let result = s.smooth("AA:BB", (255, 255, 255));
        // After 50 steps at reference interval should be very close to target
        assert!(result.0 >= 250, "r={}", result.0);
        assert!(result.1 >= 250, "g={}", result.1);
        assert!(result.2 >= 250, "b={}", result.2);
    }

    #[test]
    fn smoother_clear_resets_state() {
        let mut s = ColorSmoother::new();
        s.smooth("AA:BB", (100, 100, 100));
        s.mark_updated();
        s.clear();
        // After clear, next sample should snap
        let result = s.smooth("AA:BB", (200, 50, 0));
        assert_eq!(result, (200, 50, 0));
    }

    #[test]
    fn smoother_tracks_bulbs_independently() {
        let mut s = ColorSmoother::new();
        let a = s.smooth("AA", (255, 0, 0));
        let b = s.smooth("BB", (0, 0, 255));
        assert_eq!(a, (255, 0, 0));
        assert_eq!(b, (0, 0, 255));
    }
}
