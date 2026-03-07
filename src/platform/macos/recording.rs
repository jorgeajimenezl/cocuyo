use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::futures::Stream;
use tokio::sync::mpsc;
use tracing::info;

use crate::app::RecordingState;
use crate::frame::FrameData;
use crate::recording::{RecordingCommand, RecordingEvent};

/// Placeholder recording subscription for macOS.
///
/// Produces black RGBA frames at a fixed resolution so the rest of the
/// application (UI, ambient lighting pipeline, GPU sampling) can be
/// developed and tested on macOS before a real capture backend is wired up.
pub fn recording_subscription(
    input: &(u64, u32),
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    let fps_limit = input.1;

    Box::pin(iced::stream::channel(2, async move |mut output| {
        use iced::futures::SinkExt;

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RecordingCommand>(1);
        output.send(RecordingEvent::Ready(cmd_tx)).await.ok();

        output
            .send(RecordingEvent::StateChanged(RecordingState::Recording))
            .await
            .ok();

        info!("macOS placeholder recording started");

        let width: u32 = 1920;
        let height: u32 = 1080;
        let black_frame = Arc::new(FrameData::Cpu {
            data: Arc::new(vec![0u8; (width * height * 4) as usize]),
            width,
            height,
        });

        let frame_interval = if fps_limit == 0 {
            Duration::from_secs_f64(1.0 / 30.0) // default 30 fps
        } else {
            Duration::from_secs_f64(1.0 / fps_limit as f64)
        };

        loop {
            let sleep = tokio::time::sleep(frame_interval);
            tokio::pin!(sleep);

            tokio::select! {
                _ = &mut sleep => {
                    if output.send(RecordingEvent::Frame(black_frame.clone())).await.is_err() {
                        break;
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(RecordingCommand::Stop) | None => {
                            info!("macOS placeholder recording stopped");
                            break;
                        }
                    }
                }
            }
        }

        output
            .send(RecordingEvent::StateChanged(RecordingState::Idle))
            .await
            .ok();

        // Keep alive so the subscription isn't restarted
        std::future::pending::<()>().await;
    }))
}
