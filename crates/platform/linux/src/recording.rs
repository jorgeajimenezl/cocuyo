use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use futures::channel::mpsc;
use tracing::{error, info};

use super::gst_pipeline::GpuBackend;
use super::stream;
use cocuyo_core::frame::FrameData;
use cocuyo_core::recording::RecordingEvent;
use cocuyo_core::recording_driver::{
    BackendHandles, RecordingBackend, ShutdownHook, StartOutcome, run_recording,
};

struct LinuxBackend {
    backend: GpuBackend,
}

impl RecordingBackend for LinuxBackend {
    fn start(&mut self) -> Pin<Box<dyn Future<Output = StartOutcome> + Send + '_>> {
        let backend = self.backend.clone();
        Box::pin(async move {
            info!(backend = %backend, "Starting recording");

            let (portal_stream, fd, session) = match stream::open_portal().await {
                Ok(result) => result,
                Err(e) => {
                    error!(error = %e, "Failed to open portal");
                    return StartOutcome::Failed(e.to_string());
                }
            };

            let node_id = portal_stream.pipe_wire_node_id();
            info!(
                node_id = node_id,
                fd = fd.as_raw_fd(),
                "PipeWire stream connected"
            );

            let (frame_tx, frame_rx) = mpsc::channel::<Arc<FrameData>>(2);

            let pw_handle =
                std::thread::spawn(move || stream::start_streaming(node_id, fd, frame_tx, backend));

            let shutdown: ShutdownHook = Box::new(move || {
                Box::pin(async move {
                    match pw_handle.join() {
                        Ok(Ok(())) => info!("PipeWire streaming ended"),
                        Ok(Err(e)) => error!(error = %e, "PipeWire streaming error"),
                        Err(_) => error!("PipeWire thread panicked"),
                    }
                    if let Err(e) = session.close().await {
                        error!(error = %e, "Failed to close portal session");
                    } else {
                        info!("Portal session closed");
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
    input: &(u64, GpuBackend, u32),
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    let backend = input.1.clone();
    let fps_limit = input.2;
    run_recording(fps_limit, LinuxBackend { backend })
}
