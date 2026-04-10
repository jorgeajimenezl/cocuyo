use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::Stream;
use futures::channel::mpsc;
use tracing::{info, warn};

use cocuyo_core::errors::RecordingError;
use cocuyo_core::frame::FrameData;
use cocuyo_core::recording::RecordingEvent;
use cocuyo_core::recording_driver::{
    BackendHandles, RecordingBackend, ShutdownHook, StartOutcome, run_recording,
};

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
    fn build_zero_copy_frame(&mut self, frame: &mut Frame) -> Option<Arc<FrameData>> {
        if self.zero_copy_failed || !dx12_import::is_d3d_shared_import_available() {
            return None;
        }

        let source_texture: &ID3D11Texture2D = frame.as_raw_texture();
        let shared_handle = match create_shared_handle_from_texture(source_texture) {
            Ok(h) => h,
            Err(e) => {
                warn!(error = %e, "CreateSharedHandle failed on WGC texture, disabling zero-copy");
                self.zero_copy_failed = true;
                return None;
            }
        };

        // Hold the capture frame so WGC doesn't reclaim the buffer slot.
        let held = frame.hold_capture_frame();
        let texture_clone: ID3D11Texture2D = frame.as_raw_texture().clone();

        Some(Arc::new(FrameData::Gpu(Box::new(HeldFrame::new(
            held,
            texture_clone,
            shared_handle,
            frame.width(),
            frame.height(),
        )))))
    }

    fn build_cpu_frame(
        &mut self,
        frame: &mut Frame,
    ) -> Result<Arc<FrameData>, windows_capture::frame::Error> {
        let mut buffer = frame.buffer()?;

        let bgra_data = if buffer.has_padding() {
            buffer.as_nopadding_buffer(&mut self.nopadding_buf).to_vec()
        } else {
            buffer.as_raw_buffer().to_vec()
        };

        Ok(Arc::new(FrameData::Cpu {
            data: bgra_data,
            width: frame.width(),
            height: frame.height(),
        }))
    }
}

fn create_shared_handle_from_texture(
    texture: &ID3D11Texture2D,
) -> Result<windows::Win32::Foundation::HANDLE, windows::core::Error> {
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
    type Error = RecordingError;

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
        let frame_data = match self.build_zero_copy_frame(frame) {
            Some(gpu_frame) => gpu_frame,
            None => self
                .build_cpu_frame(frame)
                .map_err(|e| RecordingError::StreamFailed(e.to_string()))?,
        };

        if let Err(e) = self.frame_tx.try_send(frame_data)
            && e.is_disconnected()
        {
            capture_control.stop();
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        info!("Windows capture source closed");
        Ok(())
    }
}

struct WindowsBackend {
    target: CaptureTarget,
    fps_limit: u32,
}

impl RecordingBackend for WindowsBackend {
    fn start(&mut self) -> Pin<Box<dyn Future<Output = StartOutcome> + Send + '_>> {
        let target = self.target;
        let fps_limit = self.fps_limit;
        Box::pin(async move {
            info!("Starting Windows screen capture");

            let (frame_tx, frame_rx) = mpsc::channel::<Arc<FrameData>>(2);

            // Push the FPS target down to WGC so it doesn't deliver frames
            // above the rate the driver would just drop. 0 = unlimited.
            let min_update_interval = if fps_limit == 0 {
                MinimumUpdateIntervalSettings::Default
            } else {
                MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(
                    1.0 / fps_limit as f64,
                ))
            };

            let settings = Settings::new(
                target,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Default,
                min_update_interval,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                3,
                frame_tx,
            );

            let capture_control = match CaptureHandler::start_free_threaded(settings) {
                Ok(control) => control,
                Err(e) => return StartOutcome::Failed(e.to_string()),
            };

            let shutdown: ShutdownHook = Box::new(move || {
                Box::pin(async move {
                    match tokio::task::spawn_blocking(move || capture_control.stop()).await {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => {
                            warn!("Capture stop error: {:?}", e);
                            Some(RecordingError::StreamFailed(format!("{e:?}")))
                        }
                        Err(_) => Some(RecordingError::ThreadPanicked),
                    }
                })
            });

            StartOutcome::Started(BackendHandles {
                frames: Box::pin(frame_rx),
                shutdown,
            })
        })
    }
}

pub fn recording_subscription(
    input: &(u64, CaptureTarget, u32),
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    let target = input.1;
    let fps_limit = input.2;
    run_recording(fps_limit, WindowsBackend { target, fps_limit })
}
