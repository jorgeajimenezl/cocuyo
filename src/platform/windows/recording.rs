use std::pin::Pin;
use std::sync::Arc;

use iced::futures::Stream;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::app::RecordingState;
use crate::frame::FrameData;
use crate::recording::{RecordingCommand, RecordingEvent};

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::graphics_capture_picker::GraphicsCapturePicker;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

struct CaptureHandler {
    frame_tx: mpsc::Sender<Arc<FrameData>>,
    nopadding_buf: Vec<u8>,
}

impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = mpsc::Sender<Arc<FrameData>>;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            frame_tx: ctx.flags,
            nopadding_buf: Vec::new(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let width = frame.width();
        let height = frame.height();

        let mut buffer = frame.buffer()?;

        let rgba_data = if buffer.has_padding() {
            buffer.as_nopadding_buffer(&mut self.nopadding_buf).to_vec()
        } else {
            buffer.as_raw_buffer().to_vec()
        };

        let frame_data = Arc::new(FrameData::Cpu {
            data: Arc::new(rgba_data),
            width,
            height,
        });

        match self.frame_tx.try_send(frame_data) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Backpressure: drop frame, UI is behind
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Receiver dropped, stop capture
                capture_control.stop();
            }
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        info!("Windows capture source closed");
        Ok(())
    }
}

/// Result of the picker dialog, sent from the capture thread to the subscription.
enum PickResult {
    /// User selected a capture target; capture has started.
    Started,
    /// User cancelled the picker dialog.
    Cancelled,
    /// An error occurred.
    Error(String),
}

pub fn recording_subscription(_input: &u64) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    Box::pin(iced::stream::channel(2, async move |mut output| {
        use iced::futures::SinkExt;

        // Create command channel so the app can signal stop
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RecordingCommand>(1);

        output.send(RecordingEvent::Ready(cmd_tx)).await.ok();

        output
            .send(RecordingEvent::StateChanged(RecordingState::Starting))
            .await
            .ok();

        info!("Starting Windows screen capture");

        // Create bounded channel for frames from capture thread
        let (frame_tx, mut frame_rx) = mpsc::channel::<Arc<FrameData>>(2);

        // Oneshot to know whether the picker succeeded
        let (pick_tx, pick_rx) = tokio::sync::oneshot::channel::<PickResult>();

        // Spawn a dedicated thread that runs both the picker dialog and the
        // capture loop. PickedGraphicsCaptureItem is !Send so both operations
        // must happen on the same thread.
        let capture_handle = std::thread::spawn(move || {
            let item = match GraphicsCapturePicker::pick_item() {
                Ok(Some(item)) => item,
                Ok(None) => {
                    let _ = pick_tx.send(PickResult::Cancelled);
                    return;
                }
                Err(e) => {
                    let _ = pick_tx.send(PickResult::Error(e.to_string()));
                    return;
                }
            };

            // Signal that the picker succeeded; capture is about to start
            let _ = pick_tx.send(PickResult::Started);

            let settings = Settings::new(
                item,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Rgba8,
                frame_tx,
            );

            // start() blocks this thread until capture ends.
            // Capture ends when the handler calls capture_control.stop()
            // (triggered by the frame channel closing).
            if let Err(e) = CaptureHandler::start(settings) {
                warn!(error = %e, "Capture error");
            }
        });

        // Wait for the picker result
        match pick_rx.await {
            Ok(PickResult::Started) => {}
            Ok(PickResult::Cancelled) => {
                info!("Capture picker cancelled");
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Idle))
                    .await
                    .ok();
                std::future::pending::<()>().await;
                return;
            }
            Ok(PickResult::Error(msg)) => {
                error!(error = %msg, "Capture picker error");
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Error(msg)))
                    .await
                    .ok();
                std::future::pending::<()>().await;
                return;
            }
            Err(_) => {
                error!("Capture thread exited before sending pick result");
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Error(
                        "Capture thread exited unexpectedly".to_string(),
                    )))
                    .await
                    .ok();
                std::future::pending::<()>().await;
                return;
            }
        }

        output
            .send(RecordingEvent::StateChanged(RecordingState::Recording))
            .await
            .ok();

        // Forward frames until capture finishes or we receive a stop command
        loop {
            tokio::select! {
                frame = frame_rx.recv() => {
                    match frame {
                        Some(frame) => {
                            if output.send(RecordingEvent::Frame(frame)).await.is_err() {
                                break;
                            }
                        }
                        None => {
                            // Capture thread finished (sender dropped)
                            break;
                        }
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(RecordingCommand::Stop) | None => {
                            info!("Stop command received, shutting down capture");
                            // Drop frame_rx to close the channel — the handler
                            // will detect Closed on its next try_send and call
                            // capture_control.stop(), which breaks the message
                            // loop and lets start() return.
                            drop(frame_rx);
                            // Wait for the capture thread to finish
                            let _ = tokio::task::spawn_blocking(move || {
                                if let Err(e) = capture_handle.join() {
                                    warn!("Capture thread panicked: {:?}", e);
                                }
                            })
                            .await;
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
