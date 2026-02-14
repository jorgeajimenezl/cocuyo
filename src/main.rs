use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

use tracing::{error, info};

mod app;
mod dmabuf_handler;
mod formats;
mod gst_pipeline;
mod screen;
mod stream;
mod theme;
mod vulkan_dmabuf;
mod widget;

use app::{Cocuyo, FrameData, RecordingState};
use gst_pipeline::{GpuBackend, detect_available_backends};

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    gstreamer::init().expect("Failed to initialize GStreamer");
    info!("GStreamer initialized");

    pipewire::init();

    let available_backends = detect_available_backends();
    info!(
        backends = ?available_backends.iter().map(|b| b.to_string()).collect::<Vec<_>>(),
        "Detected GPU backends"
    );

    let recording_state = Arc::new(Mutex::new(RecordingState::Idle));
    let (start_recording_tx, start_recording_rx) = std::sync::mpsc::channel::<((), GpuBackend)>();
    let mainloop_quit: Arc<Mutex<Option<stream::MainLoopQuitHandle>>> = Arc::new(Mutex::new(None));

    let (frame_sender, frame_receiver) = tokio::sync::mpsc::unbounded_channel();
    let frame_receiver = Arc::new(Mutex::new(frame_receiver));

    spawn_recording_thread(
        start_recording_rx,
        mainloop_quit.clone(),
        recording_state.clone(),
        frame_sender,
    );

    let fr = frame_receiver.clone();
    let rs = recording_state.clone();
    let ml = mainloop_quit.clone();
    let ab = available_backends.clone();

    iced::daemon(
        move || Cocuyo::new(fr.clone(), rs.clone(), start_recording_tx.clone(), ml.clone(), ab.clone()),
        Cocuyo::update,
        Cocuyo::view,
    )
    .title(Cocuyo::title)
    .theme(Cocuyo::theme)
    .subscription(Cocuyo::subscription)
    .font(include_bytes!("../assets/fonts/Geist-Regular.otf").as_slice())
    .font(include_bytes!("../assets/fonts/GeistPixel-Circle.otf").as_slice())
    .default_font(iced::Font::with_name("Geist"))
    .run()
}

fn spawn_recording_thread(
    start_recording_rx: std::sync::mpsc::Receiver<((), GpuBackend)>,
    mainloop_quit: Arc<Mutex<Option<stream::MainLoopQuitHandle>>>,
    recording_state: Arc<Mutex<RecordingState>>,
    frame_sender: tokio::sync::mpsc::UnboundedSender<FrameData>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

        while let Ok(((), selected_backend)) = start_recording_rx.recv() {
            *recording_state.lock().unwrap() = RecordingState::Starting;

            info!(backend = %selected_backend, "Starting recording");

            let result = rt.block_on(stream::open_portal());

            match result {
                Ok((portal_stream, fd, session)) => {
                    let node_id = portal_stream.pipe_wire_node_id();

                    info!(
                        node_id = node_id,
                        fd = fd.as_raw_fd(),
                        "PipeWire stream connected"
                    );

                    *recording_state.lock().unwrap() = RecordingState::Recording;

                    let sender = frame_sender.clone();

                    if let Err(e) =
                        stream::start_streaming(node_id, fd, sender, mainloop_quit.clone(), selected_backend)
                    {
                        error!(error = %e, "PipeWire streaming error");
                        *recording_state.lock().unwrap() = RecordingState::Error(e.to_string());
                    } else {
                        *recording_state.lock().unwrap() = RecordingState::Idle;
                    }

                    if let Err(e) = rt.block_on(session.close()) {
                        error!(error = %e, "Failed to close portal session");
                    } else {
                        info!("Portal session closed");
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
