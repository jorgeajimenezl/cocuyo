//! Shared driver for platform recording subscriptions.
//!
//! Each platform capture backend (PipeWire, Windows Graphics Capture,
//! ScreenCaptureKit) only needs to implement [`RecordingBackend`] — the
//! lifecycle, FPS gating, command handling, and shutdown orchestration live
//! here so the three `recording.rs` files no longer need to duplicate them
//! (or depend on `iced`).
//!
//! The returned stream is a plain [`futures::Stream`]; the binary wraps it
//! with `iced::Subscription::run_with` at the call site.

use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc as futures_mpsc;
use futures::sink::SinkExt;
use futures::stream::{self, Stream, StreamExt};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::frame::FrameData;
use crate::recording::{RecordingCommand, RecordingEvent, RecordingState};

/// Future returned by a backend's shutdown hook.
pub type ShutdownFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Cleanup closure invoked exactly once when the recording loop exits
/// (either because the user issued `Stop` or the capture source ended).
pub type ShutdownHook = Box<dyn FnOnce() -> ShutdownFuture + Send>;

/// Everything the driver needs from a backend once setup has succeeded.
pub struct BackendHandles {
    /// Channel receiving frames from the platform capture thread/task.
    pub frame_rx: mpsc::Receiver<Arc<FrameData>>,
    /// Cleanup hook (stop capture, join threads, close sessions).
    pub shutdown: ShutdownHook,
}

/// Outcome of [`RecordingBackend::start`].
pub enum StartOutcome {
    /// Capture started successfully.
    Started(BackendHandles),
    /// User cancelled setup (e.g. dismissed a picker). Driver will emit
    /// `RecordingState::Idle` and keep the subscription alive.
    Cancelled,
    /// Setup failed. Driver will emit `RecordingState::Error(msg)` and keep
    /// the subscription alive.
    Failed(String),
}

/// Platform-specific capture setup. Implementors own all platform state.
pub trait RecordingBackend: Send + 'static {
    /// Perform platform setup. The returned future must be `Send`.
    fn start(&mut self) -> Pin<Box<dyn Future<Output = StartOutcome> + Send + '_>>;
}

/// Build a recording event stream from a [`RecordingBackend`].
///
/// Handles the full lifecycle:
/// `Ready` → `Starting` → backend setup → `Recording` → frame loop with FPS
/// gating → shutdown → `Idle` → pending.
pub fn run_recording<B: RecordingBackend>(
    fps_limit: u32,
    mut backend: B,
) -> Pin<Box<dyn Stream<Item = RecordingEvent> + Send>> {
    Box::pin(channel(2, async move |mut sender| {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RecordingCommand>(1);
        sender.send(RecordingEvent::Ready(cmd_tx)).await.ok();
        sender
            .send(RecordingEvent::StateChanged(RecordingState::Starting))
            .await
            .ok();

        let handles = match backend.start().await {
            StartOutcome::Started(h) => h,
            StartOutcome::Cancelled => {
                info!("Recording setup cancelled");
                sender
                    .send(RecordingEvent::StateChanged(RecordingState::Idle))
                    .await
                    .ok();
                std::future::pending::<()>().await;
                return;
            }
            StartOutcome::Failed(msg) => {
                error!(error = %msg, "Recording backend failed to start");
                sender
                    .send(RecordingEvent::StateChanged(RecordingState::Error(msg)))
                    .await
                    .ok();
                std::future::pending::<()>().await;
                return;
            }
        };

        let BackendHandles {
            mut frame_rx,
            shutdown,
        } = handles;
        let mut shutdown = Some(shutdown);

        sender
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
                            if let Some(interval) = frame_interval
                                && let Some(last) = last_forwarded
                                && last.elapsed() < interval
                            {
                                continue;
                            }
                            last_forwarded = Some(Instant::now());
                            if sender.send(RecordingEvent::Frame(frame)).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(RecordingCommand::Stop) | None => {
                            info!("Stop command received, shutting down recording");
                            drop(frame_rx);
                            if let Some(sd) = shutdown.take() {
                                sd().await;
                            }
                            break;
                        }
                    }
                }
            }
        }

        if let Some(sd) = shutdown.take() {
            sd().await;
        }

        sender
            .send(RecordingEvent::StateChanged(RecordingState::Idle))
            .await
            .ok();

        // Keep the subscription alive so iced doesn't restart it.
        std::future::pending::<()>().await;
    }))
}

/// Build a `Stream` from an async closure that pushes items into an mpsc
/// sender. Mirrors `iced::stream::channel` but lives in core so platform
/// crates don't need an iced dependency.
pub fn channel<T>(
    size: usize,
    f: impl AsyncFnOnce(futures_mpsc::Sender<T>),
) -> impl Stream<Item = T> {
    let (sender, receiver) = futures_mpsc::channel(size);
    let runner = stream::once(f(sender)).filter_map(|_| async { None });
    stream::select(receiver, runner)
}
