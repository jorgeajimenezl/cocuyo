use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::futures::Stream;
use screencapturekit::async_api::AsyncSCStream;
use screencapturekit::content_sharing_picker::{
    AsyncSCContentSharingPicker, SCContentSharingPickerConfiguration,
    SCContentSharingPickerMode, SCPickerOutcome,
};
use screencapturekit::stream::configuration::SCStreamConfiguration;
use screencapturekit::stream::output_type::SCStreamOutputType;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::app::RecordingState;
use crate::frame::FrameData;
use crate::recording::{RecordingCommand, RecordingEvent};

/// Convert BGRA pixel data to RGBA, stripping row padding if present.
fn bgra_to_rgba(src: &[u8], width: usize, height: usize, bytes_per_row: usize) -> Vec<u8> {
    let stride = width * 4;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let row_start = row * bytes_per_row;
        let row_data = &src[row_start..row_start + stride];
        for chunk in row_data.chunks_exact(4) {
            rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
        }
    }
    rgba
}

pub fn recording_subscription(
    input: &(u64, u32),
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    let fps_limit = input.1;

    Box::pin(iced::stream::channel(2, async move |mut output| {
        use iced::futures::SinkExt;

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RecordingCommand>(1);
        output.send(RecordingEvent::Ready(cmd_tx)).await.ok();

        output
            .send(RecordingEvent::StateChanged(RecordingState::Starting))
            .await
            .ok();

        info!("Showing macOS content sharing picker");

        // Show the system content picker (like Linux's XDG Desktop Portal)
        let mut picker_config = SCContentSharingPickerConfiguration::new();
        picker_config.set_allowed_picker_modes(&[
            SCContentSharingPickerMode::SingleDisplay,
            SCContentSharingPickerMode::SingleWindow,
        ]);

        let picker = AsyncSCContentSharingPicker::new();
        let outcome = picker.show(&picker_config).await;

        let result = match outcome {
            SCPickerOutcome::Picked(result) => result,
            SCPickerOutcome::Cancelled => {
                info!("macOS content picker cancelled by user");
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Idle))
                    .await
                    .ok();
                std::future::pending::<()>().await;
                return;
            }
            SCPickerOutcome::Error(msg) => {
                warn!("macOS content picker error: {}", msg);
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Error(msg)))
                    .await
                    .ok();
                std::future::pending::<()>().await;
                return;
            }
        };

        let filter = result.filter();
        let (pixel_width, pixel_height) = result.pixel_size();
        info!(
            width = pixel_width,
            height = pixel_height,
            "macOS content picker selection received"
        );

        // Configure capture stream
        let fps = if fps_limit == 0 { 60 } else { fps_limit };
        let config = SCStreamConfiguration::new()
            .with_width(pixel_width)
            .with_height(pixel_height)
            .with_shows_cursor(false)
            .with_fps(fps);

        let mut stream = AsyncSCStream::new(&filter, &config, 2, SCStreamOutputType::Screen);

        if let Err(e) = stream.start_capture() {
            let msg = format!("Failed to start macOS capture: {e}");
            warn!("{}", msg);
            output
                .send(RecordingEvent::StateChanged(RecordingState::Error(msg)))
                .await
                .ok();
            std::future::pending::<()>().await;
            return;
        }

        output
            .send(RecordingEvent::StateChanged(RecordingState::Recording))
            .await
            .ok();

        info!("macOS screen capture started");

        // Frame rate gating (same pattern as Linux/Windows)
        let frame_interval: Option<Duration> = if fps_limit == 0 {
            None
        } else {
            Some(Duration::from_secs_f64(1.0 / fps_limit as f64))
        };
        let mut last_forwarded: Option<Instant> = None;

        loop {
            tokio::select! {
                sample = stream.next() => {
                    let Some(sample) = sample else {
                        // Stream ended
                        info!("macOS capture stream ended");
                        break;
                    };

                    // FPS gating
                    if let Some(interval) = frame_interval {
                        if let Some(last) = last_forwarded {
                            if last.elapsed() < interval {
                                continue;
                            }
                        }
                    }
                    last_forwarded = Some(Instant::now());

                    let Some(pixel_buffer) = sample.image_buffer() else {
                        continue;
                    };

                    let guard = match pixel_buffer.lock_read_only() {
                        Ok(g) => g,
                        Err(e) => {
                            warn!("Failed to lock pixel buffer: {}", e);
                            continue;
                        }
                    };

                    let w = guard.width() as u32;
                    let h = guard.height() as u32;
                    let bpr = guard.bytes_per_row();
                    let src = guard.as_slice();

                    let rgba = bgra_to_rgba(src, w as usize, h as usize, bpr);

                    let frame = Arc::new(FrameData::Cpu {
                        data: Arc::new(rgba),
                        width: w,
                        height: h,
                    });

                    if output.send(RecordingEvent::Frame(frame)).await.is_err() {
                        break;
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(RecordingCommand::Stop) | None => {
                            info!("Stop command received, shutting down macOS capture");
                            if let Err(e) = stream.stop_capture() {
                                warn!("Capture stop error: {:?}", e);
                            }
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
