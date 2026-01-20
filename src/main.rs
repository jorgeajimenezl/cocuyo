use std::os::fd::IntoRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use iced::{Element, Subscription, Task, Theme, window};
use pipewire as pw;
use tracing::{error, info};

mod app;
mod dmabuf_handler;
mod formats;
mod gst_pipeline;
mod stream;

use app::{CocuyoApp, Message, RecordingState};
use gst_pipeline::{GpuBackend, detect_available_backends};

// Global state for initialization (set once before daemon starts)
static INIT_STATE: OnceLock<InitState> = OnceLock::new();

#[derive(Debug)]
struct InitState {
    frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<app::FrameData>>>,
    recording_state: Arc<Mutex<RecordingState>>,
    start_recording_tx: std::sync::mpsc::Sender<((), GpuBackend)>,
    stop_flag: Arc<AtomicBool>,
    available_backends: Vec<GpuBackend>,
}

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    gstreamer::init().expect("Failed to initialize GStreamer");
    info!("GStreamer initialized");

    pw::init();

    let available_backends = detect_available_backends();
    info!(
        backends = ?available_backends.iter().map(|b| b.to_string()).collect::<Vec<_>>(),
        "Detected GPU backends"
    );

    let recording_state = Arc::new(Mutex::new(RecordingState::Idle));
    let (start_recording_tx, start_recording_rx) = std::sync::mpsc::channel::<((), GpuBackend)>();
    let stop_flag = Arc::new(AtomicBool::new(false));

    let (frame_sender, frame_receiver) = tokio::sync::mpsc::unbounded_channel();
    let frame_receiver = Arc::new(Mutex::new(frame_receiver));

    spawn_recording_thread(
        start_recording_rx,
        stop_flag.clone(),
        recording_state.clone(),
        frame_sender,
    );

    // Store initialization state globally
    INIT_STATE
        .set(InitState {
            frame_receiver,
            recording_state,
            start_recording_tx,
            stop_flag,
            available_backends,
        })
        .expect("Failed to set init state");

    info!("Using wgpu backend with iced");

    iced::daemon(boot, AppWrapper::update, AppWrapper::view)
        .subscription(AppWrapper::subscription)
        .title(AppWrapper::title)
        .theme(AppWrapper::theme)
        .run()
}

fn boot() -> (AppWrapper, Task<Message>) {
    let init = INIT_STATE.get().expect("Init state not set");

    let (app, task) = CocuyoApp::new(
        init.frame_receiver.clone(),
        init.recording_state.clone(),
        init.start_recording_tx.clone(),
        init.stop_flag.clone(),
        init.available_backends.clone(),
    );

    (AppWrapper { app }, task)
}

struct AppWrapper {
    app: CocuyoApp,
}

impl AppWrapper {
    fn title(&self, window_id: window::Id) -> String {
        self.app.title(window_id)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        self.app.update(message)
    }

    fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        self.app.view(window_id)
    }

    fn subscription(&self) -> Subscription<Message> {
        self.app.subscription()
    }

    fn theme(&self, window_id: window::Id) -> Theme {
        self.app.theme(window_id)
    }
}

fn spawn_recording_thread(
    start_recording_rx: std::sync::mpsc::Receiver<((), GpuBackend)>,
    stop_flag: Arc<AtomicBool>,
    recording_state: Arc<Mutex<RecordingState>>,
    frame_sender: tokio::sync::mpsc::UnboundedSender<app::FrameData>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

        while let Ok(((), selected_backend)) = start_recording_rx.recv() {
            stop_flag.store(false, Ordering::SeqCst);
            *recording_state.lock().unwrap() = RecordingState::Starting;

            info!(backend = %selected_backend, "Starting recording");

            let result = rt.block_on(stream::open_portal());

            match result {
                Ok((portal_stream, fd)) => {
                    let node_id = portal_stream.pipe_wire_node_id();

                    info!(
                        node_id = node_id,
                        fd = fd.try_clone().unwrap().into_raw_fd(),
                        "PipeWire stream connected"
                    );

                    *recording_state.lock().unwrap() = RecordingState::Recording;

                    let sender = frame_sender.clone();
                    let stop = stop_flag.clone();

                    if let Err(e) =
                        stream::start_streaming(node_id, fd, sender, stop, selected_backend)
                    {
                        error!(error = %e, "PipeWire streaming error");
                        *recording_state.lock().unwrap() = RecordingState::Error(e.to_string());
                    } else {
                        *recording_state.lock().unwrap() = RecordingState::Idle;
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to open portal");
                    *recording_state.lock().unwrap() = RecordingState::Error(e.to_string());
                }
            }
        }
    });
}
