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

/// Stream of captured frames produced by a backend.
pub type FrameStream = Pin<Box<dyn Stream<Item = Arc<FrameData>> + Send>>;

/// Everything the driver needs from a backend once setup has succeeded.
pub struct BackendHandles {
    /// Frames produced by the platform capture source. The driver polls
    /// this directly — backends are free to return any `Stream`, whether
    /// it's a channel receiver or a direct adapter over the capture API.
    pub frames: FrameStream,
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
            mut frames,
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
                frame = frames.next() => {
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
                            // Drop the frame stream so the capture source notices
                            // (e.g. channel closes, Arc refcount drops).
                            drop(frames);
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

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use futures::channel::mpsc as futures_mpsc;
    use futures::StreamExt as _;

    use super::*;
    use crate::frame::FrameData;
    use crate::recording::{RecordingCommand, RecordingEvent, RecordingState};

    fn cpu_frame() -> Arc<FrameData> {
        Arc::new(FrameData::Cpu {
            data: vec![0u8; 4],
            width: 1,
            height: 1,
        })
    }

    /// Create `BackendHandles` wired to a controllable frame sender and a
    /// shutdown flag that is set when the hook fires.
    fn started_handles() -> (
        futures_mpsc::Sender<Arc<FrameData>>,
        Arc<AtomicBool>,
        BackendHandles,
    ) {
        let (frame_tx, frame_rx) = futures_mpsc::channel(8);
        let shutdown_called = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown_called);
        let handles = BackendHandles {
            frames: Box::pin(frame_rx),
            shutdown: Box::new(move || {
                flag.store(true, Ordering::SeqCst);
                Box::pin(async {})
            }),
        };
        (frame_tx, shutdown_called, handles)
    }

    struct MockBackend(Option<StartOutcome>);

    impl RecordingBackend for MockBackend {
        fn start(&mut self) -> Pin<Box<dyn Future<Output = StartOutcome> + Send + '_>> {
            let outcome = self.0.take().expect("start() called twice");
            Box::pin(async move { outcome })
        }
    }

    /// Full happy-path: Ready → Starting → Recording → Frame → Stop → Idle.
    /// Verifies the shutdown hook fires.
    #[tokio::test]
    async fn test_successful_lifecycle() {
        let (mut frame_tx, shutdown_called, handles) = started_handles();
        let mut stream = run_recording(0, MockBackend(Some(StartOutcome::Started(handles))));

        let RecordingEvent::Ready(cmd_tx) = stream.next().await.unwrap() else {
            panic!("expected Ready");
        };

        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Starting)
        ));
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Recording)
        ));

        frame_tx.try_send(cpu_frame()).unwrap();
        assert!(matches!(stream.next().await.unwrap(), RecordingEvent::Frame(_)));

        cmd_tx.send(RecordingCommand::Stop).await.unwrap();

        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Idle)
        ));
        assert!(shutdown_called.load(Ordering::SeqCst));
    }

    /// Backend returns `Cancelled` → Ready, Starting, Idle events. Stream
    /// stays alive (pending) so the subscription isn't restarted.
    #[tokio::test]
    async fn test_cancelled_backend() {
        let mut stream = run_recording(0, MockBackend(Some(StartOutcome::Cancelled)));

        assert!(matches!(stream.next().await.unwrap(), RecordingEvent::Ready(_)));
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Starting)
        ));
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Idle)
        ));

        // After Idle the driver calls `pending()` — next poll must not resolve.
        let timed_out = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            stream.next(),
        )
        .await;
        assert!(timed_out.is_err(), "stream should be pending after Cancelled");
    }

    /// Backend returns `Failed` → Ready, Starting, Error(msg) events. Stream
    /// stays alive afterwards.
    #[tokio::test]
    async fn test_failed_backend() {
        let mut stream =
            run_recording(0, MockBackend(Some(StartOutcome::Failed("oops".into()))));

        assert!(matches!(stream.next().await.unwrap(), RecordingEvent::Ready(_)));
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Starting)
        ));

        let ev = stream.next().await.unwrap();
        assert!(
            matches!(&ev, RecordingEvent::StateChanged(RecordingState::Error(m)) if m == "oops"),
            "unexpected event: {:?}",
            ev
        );

        let timed_out = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            stream.next(),
        )
        .await;
        assert!(timed_out.is_err(), "stream should be pending after Failed");
    }

    /// With `fps_limit = 1` (1-second interval), frames sent within the
    /// interval after the first are dropped and never emitted.
    #[tokio::test]
    async fn test_fps_gating() {
        let (mut frame_tx, _shutdown, handles) = started_handles();
        // fps_limit = 1 → 1-second interval, so rapid frames 2 & 3 are dropped.
        let mut stream = run_recording(1, MockBackend(Some(StartOutcome::Started(handles))));

        // Hold cmd_tx so the command channel stays open; dropping it would
        // cause cmd_rx.recv() to return None and trigger a spurious shutdown.
        let RecordingEvent::Ready(_cmd_tx) = stream.next().await.unwrap() else {
            panic!("expected Ready");
        };
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Starting)
        ));
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Recording)
        ));

        // First frame — always forwarded (last_forwarded is None).
        frame_tx.try_send(cpu_frame()).unwrap();
        assert!(matches!(stream.next().await.unwrap(), RecordingEvent::Frame(_)));

        // Two more frames sent immediately — both within the 1-second interval.
        frame_tx.try_send(cpu_frame()).unwrap();
        frame_tx.try_send(cpu_frame()).unwrap();

        // Close the frame stream; driver exits without emitting the gated frames.
        drop(frame_tx);

        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Idle)
        ));
    }

    /// Dropping the frame sender (no `Stop` command) causes the driver to exit
    /// the loop naturally, call the shutdown hook, and emit Idle.
    #[tokio::test]
    async fn test_stream_ends_when_frames_close() {
        let (frame_tx, shutdown_called, handles) = started_handles();
        let mut stream = run_recording(0, MockBackend(Some(StartOutcome::Started(handles))));

        let RecordingEvent::Ready(_cmd_tx) = stream.next().await.unwrap() else {
            panic!("expected Ready");
        };
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Starting)
        ));
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Recording)
        ));

        drop(frame_tx); // signals end-of-stream to the driver

        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Idle)
        ));
        assert!(shutdown_called.load(Ordering::SeqCst));
    }

    /// With `fps_limit = 0` every frame is forwarded regardless of timing.
    #[tokio::test]
    async fn test_zero_fps_limit_forwards_all() {
        let (mut frame_tx, _shutdown, handles) = started_handles();
        let mut stream = run_recording(0, MockBackend(Some(StartOutcome::Started(handles))));

        let RecordingEvent::Ready(_cmd_tx) = stream.next().await.unwrap() else {
            panic!("expected Ready");
        };
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Starting)
        ));
        assert!(matches!(
            stream.next().await.unwrap(),
            RecordingEvent::StateChanged(RecordingState::Recording)
        ));

        for _ in 0..3 {
            frame_tx.try_send(cpu_frame()).unwrap();
            assert!(matches!(stream.next().await.unwrap(), RecordingEvent::Frame(_)));
        }
    }
}
