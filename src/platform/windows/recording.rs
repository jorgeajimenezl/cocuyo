use std::pin::Pin;
use std::sync::Arc;

use iced::futures::Stream;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::app::RecordingState;
use crate::frame::FrameData;
use crate::platform::windows::capture_target::CaptureTarget;
use crate::recording::{RecordingCommand, RecordingEvent};

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
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

pub fn recording_subscription(
    input: &(u64, CaptureTarget),
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    let target = input.1;

    Box::pin(iced::stream::channel(2, async move |mut output| {
        use iced::futures::SinkExt;

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RecordingCommand>(1);
        output.send(RecordingEvent::Ready(cmd_tx)).await.ok();

        output
            .send(RecordingEvent::StateChanged(RecordingState::Starting))
            .await
            .ok();

        info!("Starting Windows screen capture");

        let (frame_tx, mut frame_rx) = mpsc::channel::<Arc<FrameData>>(2);

        let settings = Settings::new(
            target,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            frame_tx,
        );

        let capture_control = match CaptureHandler::start_free_threaded(settings) {
            Ok(control) => control,
            Err(e) => {
                error!(error = %e, "Failed to start capture");
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Error(
                        e.to_string(),
                    )))
                    .await
                    .ok();
                std::future::pending::<()>().await;
                return;
            }
        };

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
                            // Drop frame_rx so the handler sees Closed on next try_send
                            drop(frame_rx);
                            // Stop the capture thread and wait for it to finish
                            let _ = tokio::task::spawn_blocking(move || {
                                if let Err(e) = capture_control.stop() {
                                    warn!("Capture stop error: {:?}", e);
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
