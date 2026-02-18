use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::sync::Arc;

use iced::futures::Stream;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::app::RecordingState;
use crate::frame::FrameData;
use crate::platform::linux::gst_pipeline::GpuBackend;
use crate::platform::linux::stream;

/// Commands sent from the app to the recording subscription.
#[derive(Debug)]
pub enum RecordingCommand {
    Stop,
}

/// Events sent from the recording subscription to the app.
#[derive(Debug, Clone)]
pub enum RecordingEvent {
    /// The subscription is ready and provides a command sender for control.
    Ready(mpsc::Sender<RecordingCommand>),
    StateChanged(RecordingState),
    Frame(Arc<FrameData>),
}

pub fn recording_subscription(
    input: &(u64, GpuBackend),
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    let backend = input.1.clone();

    Box::pin(iced::stream::channel(2, async move |mut output| {
        use iced::futures::SinkExt;

        // Create command channel so the app can signal stop
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RecordingCommand>(1);

        output
            .send(RecordingEvent::Ready(cmd_tx))
            .await
            .ok();

        output
            .send(RecordingEvent::StateChanged(RecordingState::Starting))
            .await
            .ok();

        info!(backend = %backend, "Starting recording");

        let portal_result = stream::open_portal().await;

        let (portal_stream, fd, session) = match portal_result {
            Ok(result) => result,
            Err(e) => {
                error!(error = %e, "Failed to open portal");
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Error(
                        e.to_string(),
                    )))
                    .await
                    .ok();
                // Keep alive so the subscription isn't restarted
                std::future::pending::<()>().await;
                return;
            }
        };

        let node_id = portal_stream.pipe_wire_node_id();
        info!(
            node_id = node_id,
            fd = fd.as_raw_fd(),
            "PipeWire stream connected"
        );

        output
            .send(RecordingEvent::StateChanged(RecordingState::Recording))
            .await
            .ok();

        // Create bounded channel for frames from PipeWire thread
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Arc<FrameData>>(2);

        // Spawn PipeWire thread
        let pw_handle = std::thread::spawn(move || {
            stream::start_streaming(node_id, fd, frame_tx, backend)
        });

        // Forward frames until PipeWire thread finishes or we receive a stop command.
        // On stop: drop frame_rx to close the channel, causing the PipeWire thread
        // to detect Closed on its next try_send and call mainloop.quit().
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
                            // PipeWire thread finished (sender dropped)
                            break;
                        }
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(RecordingCommand::Stop) | None => {
                            info!("Stop command received, shutting down recording");
                            // Drop receiver to signal PipeWire thread via channel close
                            drop(frame_rx);
                            break;
                        }
                    }
                }
            }
        }

        // Wait for PipeWire thread to complete
        match pw_handle.join() {
            Ok(Ok(())) => info!("PipeWire streaming ended"),
            Ok(Err(e)) => {
                error!(error = %e, "PipeWire streaming error");
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Error(
                        e.to_string(),
                    )))
                    .await
                    .ok();
            }
            Err(_) => {
                error!("PipeWire thread panicked");
                output
                    .send(RecordingEvent::StateChanged(RecordingState::Error(
                        "PipeWire thread panicked".to_string(),
                    )))
                    .await
                    .ok();
            }
        }

        // Close portal session
        if let Err(e) = session.close().await {
            error!(error = %e, "Failed to close portal session");
        } else {
            info!("Portal session closed");
        }

        output
            .send(RecordingEvent::StateChanged(RecordingState::Idle))
            .await
            .ok();

        // Keep alive so the subscription isn't restarted
        std::future::pending::<()>().await;
    }))
}
