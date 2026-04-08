use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::futures::Stream;
use screencapturekit::async_api::AsyncSCContentSharingPicker;
use screencapturekit::async_api::AsyncSCStream;
use screencapturekit::content_sharing_picker::{
    SCContentSharingPickerConfiguration, SCContentSharingPickerMode, SCPickerOutcome,
};
use screencapturekit::stream::configuration::SCStreamConfiguration;
use screencapturekit::stream::configuration::pixel_format::PixelFormat;
use screencapturekit::stream::output_type::SCStreamOutputType;
use tokio::sync::mpsc;
use tracing::{info, warn};

use cocuyo_core::frame::FrameData;
use cocuyo_core::recording::{RecordingCommand, RecordingEvent, RecordingState};

use crate::iosurface_frame::{IOSurfaceFrame, strip_stride_padding};

/// Build a `FrameData` from a captured sample.
///
/// Tries the zero-copy IOSurface path first (sends the IOSurface directly to
/// the shader widget, which imports it as a Metal texture in `prepare()`).
/// Falls back to CPU BGRA copy when IOSurface is unavailable.
fn build_frame(pixel_buffer: &screencapturekit::CVPixelBuffer) -> Option<Arc<FrameData>> {
    // Zero-copy path: send the IOSurface to the shader widget.
    // The widget imports it as a Metal texture in prepare(), wrapped in
    // autoreleasepool to avoid Cocoa run-loop re-entrancy.
    if super::metal_import::is_iosurface_import_available() {
        if let Some(surface) = pixel_buffer.io_surface() {
            let w = surface.width() as u32;
            let h = surface.height() as u32;
            if w > 0 && h > 0 {
                return Some(Arc::new(FrameData::Gpu(Arc::new(IOSurfaceFrame {
                    surface,
                    width: w,
                    height: h,
                }))));
            }
        }
    }

    // CPU fallback: lock pixel buffer and copy BGRA data
    let guard = match pixel_buffer.lock_read_only() {
        Ok(g) => g,
        Err(e) => {
            warn!("Failed to lock pixel buffer: {}", e);
            return None;
        }
    };

    let w = guard.width() as u32;
    let h = guard.height() as u32;
    let bpr = guard.bytes_per_row();
    let src = guard.as_slice();

    let bgra = strip_stride_padding(src, w as usize, h as usize, bpr);

    Some(Arc::new(FrameData::Cpu {
        data: Arc::new(bgra),
        width: w,
        height: h,
    }))
}

pub fn recording_subscription(
    input: &(u64, u32, u32),
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    let fps_limit = input.1;
    let resolution_scale = input.2;

    Box::pin(iced::stream::channel(2, async move |mut output| {
        use iced::futures::SinkExt;

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RecordingCommand>(1);
        output.send(RecordingEvent::Ready(cmd_tx)).await.ok();

        output
            .send(RecordingEvent::StateChanged(RecordingState::Starting))
            .await
            .ok();

        info!("Showing macOS content sharing picker");

        let mut picker_config = SCContentSharingPickerConfiguration::new();
        picker_config.set_allowed_picker_modes(&[
            SCContentSharingPickerMode::SingleDisplay,
            SCContentSharingPickerMode::SingleWindow,
        ]);

        let outcome = AsyncSCContentSharingPicker::show(&picker_config).await;

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
        let scale = resolution_scale.max(25).min(100);
        let scaled_w = if scale >= 100 {
            pixel_width
        } else {
            (pixel_width as u64 * scale as u64 / 100) as u32
        };
        let scaled_h = if scale >= 100 {
            pixel_height
        } else {
            (pixel_height as u64 * scale as u64 / 100) as u32
        };

        let fps = if fps_limit == 0 { 60 } else { fps_limit };
        let config = SCStreamConfiguration::new()
            .with_width(scaled_w)
            .with_height(scaled_h)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(false)
            .with_fps(fps);

        let stream = AsyncSCStream::new(&filter, &config, 2, SCStreamOutputType::Screen);

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

                    let Some(frame) = build_frame(&pixel_buffer) else {
                        continue;
                    };

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
