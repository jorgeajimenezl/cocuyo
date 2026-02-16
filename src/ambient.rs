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

/// Distribute bulb sample positions evenly across frame width, vertically centered.
fn compute_bulb_positions(num_bulbs: usize, width: u32, height: u32) -> Vec<(u32, u32)> {
    (0..num_bulbs)
        .map(|i| {
            let x = width * (i as u32 + 1) / (num_bulbs as u32 + 1);
            let y = height / 2;
            (x, y)
        })
        .collect()
}

/// Send RGB colors to WiZ bulbs concurrently.
async fn send_colors_to_bulbs(targets: Vec<(IpAddr, u8, u8, u8)>) {
    let futs: Vec<_> = targets
        .into_iter()
        .map(|(ip, r, g, b)| async move {
            let ipv4 = match ip {
                IpAddr::V4(v4) => v4,
                IpAddr::V6(_) => {
                    tracing::warn!(%ip, "Skipping non-IPv4 bulb");
                    return;
                }
            };
            let light = wiz_lights_rs::Light::new(ipv4, None);
            let mut payload = wiz_lights_rs::Payload::new();
            payload.color(&wiz_lights_rs::Color::rgb(r, g, b));
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

/// Sample frame colors at bulb positions and build a target list for sending.
/// Returns `None` if no valid samples could be taken (e.g. DmaBuf frame).
pub fn sample_frame_for_bulbs(
    frame: &Arc<FrameData>,
    selected_macs: &[String],
    bulbs: &[BulbInfo],
) -> Option<Vec<(IpAddr, u8, u8, u8)>> {
    let positions = compute_bulb_positions(selected_macs.len(), frame.width(), frame.height());
    let mut targets = Vec::new();

    for (i, mac) in selected_macs.iter().enumerate() {
        if let Some(&(x, y)) = positions.get(i) {
            if let Some((r, g, b)) = frame.sample_pixel(x, y) {
                if let Some(bulb) = bulbs.iter().find(|b| &b.mac == mac) {
                    targets.push((bulb.ip, r, g, b));
                }
            }
        }
    }

    if targets.is_empty() {
        None
    } else {
        Some(targets)
    }
}

/// Dispatch sampled colors to bulbs. Returns a future suitable for `Task::perform`.
pub async fn dispatch_bulb_colors(targets: Vec<(IpAddr, u8, u8, u8)>) {
    send_colors_to_bulbs(targets).await;
}
