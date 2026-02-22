use std::pin::Pin;
use std::sync::Arc;

use iced::futures::Stream;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::app::RecordingState;
use crate::frame::FrameData;
use crate::platform::windows::capture_target::CaptureTarget;
use crate::platform::windows::dx12_import;
use crate::platform::windows::shared_texture::SharedTexturePool;
use crate::recording::{RecordingCommand, RecordingEvent};

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
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
    pool: Option<SharedTexturePool>,
    zero_copy_failed: bool,
}

impl CaptureHandler {
    /// Try the zero-copy path: copy WGC texture → shared texture → send D3DShared.
    /// Returns `true` if the frame was sent successfully, `false` to fall through to CPU.
    fn try_zero_copy(&mut self, frame: &mut Frame, width: u32, height: u32) -> bool {
        if self.zero_copy_failed || !dx12_import::is_d3d_shared_import_available() {
            return false;
        }

        // Get the raw D3D11 texture from the capture frame.
        // Safety: the texture is valid for the duration of on_frame_arrived.
        // The returned type is from windows-capture's transitive windows crate (0.62),
        // but COM interfaces are ABI-stable across crate versions since they are
        // #[repr(transparent)] wrappers around raw COM pointers with identical vtables.
        let source_texture: &ID3D11Texture2D = unsafe {
            let raw = frame.as_raw_texture();
            &*(raw as *const _ as *const ID3D11Texture2D)
        };

        // Lazy-init the shared texture pool on first zero-copy frame
        if self.pool.is_none() {
            match SharedTexturePool::new(source_texture) {
                Ok(pool) => {
                    debug!("Shared texture pool initialized");
                    self.pool = Some(pool);
                }
                Err(e) => {
                    warn!(error = %e, "Failed to create shared texture pool, falling back to CPU");
                    self.zero_copy_failed = true;
                    return false;
                }
            }
        }

        let pool = self.pool.as_mut().unwrap();
        match pool.acquire_and_copy(source_texture, width, height) {
            Ok(Some(slot)) => {
                let frame_data = Arc::new(FrameData::D3DShared {
                    slot,
                    width,
                    height,
                });
                match self.frame_tx.try_send(frame_data) {
                    Ok(()) => true,
                    Err(mpsc::error::TrySendError::Full(_)) => true, // Backpressure, frame dropped
                    Err(mpsc::error::TrySendError::Closed(_)) => true, // Will be handled by caller
                }
            }
            Ok(None) => {
                // All slots busy (backpressure) — skip frame entirely
                true
            }
            Err(e) => {
                warn!(error = %e, "Shared texture acquire_and_copy failed, falling back to CPU");
                self.zero_copy_failed = true;
                self.pool = None;
                false
            }
        }
    }
}

impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = mpsc::Sender<Arc<FrameData>>;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            frame_tx: ctx.flags,
            nopadding_buf: Vec::new(),
            pool: None,
            zero_copy_failed: false,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let width = frame.width();
        let height = frame.height();

        // Try zero-copy path first (GPU→GPU via shared texture)
        if self.try_zero_copy(frame, width, height) {
            // Check if channel was closed
            if self.frame_tx.is_closed() {
                capture_control.stop();
            }
            return Ok(());
        }

        // CPU fallback: read pixels from GPU → system memory
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
            ColorFormat::Bgra8,
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
