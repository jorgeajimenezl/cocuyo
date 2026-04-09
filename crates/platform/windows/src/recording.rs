use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::futures::Stream;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use cocuyo_core::frame::FrameData;
use cocuyo_core::recording::{RecordingCommand, RecordingEvent, RecordingState};

use crate::held_frame::HeldFrame;

use crate::capture_target::CaptureTarget;
use crate::dx12_import;

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::{
    DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIResource1,
};
use windows::core::Interface;
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
    zero_copy_failed: bool,
}

impl CaptureHandler {
    fn try_zero_copy(&mut self, frame: &mut Frame, width: u32, height: u32) -> bool {
        if self.zero_copy_failed || !dx12_import::is_d3d_shared_import_available() {
            return false;
        }

        // Safety: COM ABI is stable across windows crate versions.
        let source_texture: &ID3D11Texture2D = frame.as_raw_texture();

        let shared_handle = match create_shared_handle_from_texture(source_texture) {
            Ok(h) => h,
            Err(e) => {
                warn!(error = %e, "CreateSharedHandle failed on WGC texture, disabling zero-copy");
                self.zero_copy_failed = true;
                return false;
            }
        };

        // Hold the capture frame so WGC doesn't reclaim the buffer slot.
        let held = frame.hold_capture_frame();

        // Safety: COM ABI is stable; clone AddRefs so the texture outlives the callback.
        let texture_clone: ID3D11Texture2D = unsafe {
            let raw = frame.as_raw_texture();
            (*(raw as *const _ as *const ID3D11Texture2D)).clone()
        };

        let frame_data = Arc::new(FrameData::Gpu(Box::new(HeldFrame::new(
            held,
            texture_clone,
            shared_handle,
            width,
            height,
        ))));

        match self.frame_tx.try_send(frame_data) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => true,
        }
    }
}

fn create_shared_handle_from_texture(
    texture: &ID3D11Texture2D,
) -> Result<windows::Win32::Foundation::HANDLE, Box<dyn std::error::Error + Send + Sync>> {
    let dxgi_resource: IDXGIResource1 = texture.cast()?;
    let handle = unsafe {
        dxgi_resource.CreateSharedHandle(
            None,
            DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0,
            None,
        )?
    };
    Ok(handle)
}

impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = mpsc::Sender<Arc<FrameData>>;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            frame_tx: ctx.flags,
            nopadding_buf: Vec::new(),
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

        if self.try_zero_copy(frame, width, height) {
            if self.frame_tx.is_closed() {
                capture_control.stop();
            }
            return Ok(());
        }

        let mut buffer = frame.buffer()?;

        let bgra_data = if buffer.has_padding() {
            buffer.as_nopadding_buffer(&mut self.nopadding_buf).to_vec()
        } else {
            buffer.as_raw_buffer().to_vec()
        };

        let frame_data = Arc::new(FrameData::Cpu {
            data: bgra_data,
            width,
            height,
        });

        match self.frame_tx.try_send(frame_data) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => {
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
    input: &(u64, CaptureTarget, u32),
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    let target = input.1;
    let fps_limit = input.2;

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
            3,
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

        let frame_interval: Option<Duration> = if fps_limit == 0 {
            None
        } else {
            Some(Duration::from_secs_f64(1.0 / fps_limit as f64))
        };
        let mut last_forwarded: Option<Instant> = None;

        loop {
            tokio::select! {
                frame = frame_rx.recv() => {
                    match frame {
                        Some(frame) => {
                            if let Some(interval) = frame_interval {
                                if let Some(last) = last_forwarded {
                                    if last.elapsed() < interval {
                                        continue;
                                    }
                                }
                            }
                            last_forwarded = Some(Instant::now());
                            if output.send(RecordingEvent::Frame(frame)).await.is_err() {
                                break;
                            }
                        }
                        None => {
                            break;
                        }
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(RecordingCommand::Stop) | None => {
                            info!("Stop command received, shutting down capture");
                            drop(frame_rx);
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

        std::future::pending::<()>().await;
    }))
}
