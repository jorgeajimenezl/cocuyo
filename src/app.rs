use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use drm_fourcc::DrmFourcc;
use iced::widget::container;
use iced::window;
use iced::{Fill, Size, Subscription, Task, Theme};

use crate::gst_pipeline::GpuBackend;
use crate::screen::WindowKind;
use crate::widget::Element;

#[derive(Clone, PartialEq)]
pub enum RecordingState {
    Idle,
    Starting,
    Recording,
    Error(String),
}

pub enum FrameData {
    DmaBuf {
        fd: OwnedFd,
        width: u32,
        height: u32,
        drm_format: DrmFourcc,
        stride: u32,
        #[allow(dead_code)]
        offset: u32,
        #[allow(dead_code)]
        modifier: u64,
    },
    Cpu {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
}

impl FrameData {
    pub fn width(&self) -> u32 {
        match self {
            FrameData::DmaBuf { width, .. } => *width,
            FrameData::Cpu { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            FrameData::DmaBuf { height, .. } => *height,
            FrameData::Cpu { height, .. } => *height,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    WindowOpened(window::Id, WindowKind),
    WindowClosed(window::Id),
    DragWindow(window::Id),
    CloseWindow(window::Id),
    MinimizeWindow(window::Id),
    MaximizeWindow(window::Id),
    OpenSettings,
    OpenPreview,
    StartRecording,
    StopRecording,
    BackendSelected(usize),
    Tick,
}

pub struct Cocuyo {
    windows: BTreeMap<window::Id, WindowKind>,
    frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<FrameData>>>,
    current_frame: Option<FrameData>,
    recording_state: Arc<Mutex<RecordingState>>,
    start_recording_tx: std::sync::mpsc::Sender<((), GpuBackend)>,
    stop_flag: Arc<AtomicBool>,
    available_backends: Vec<GpuBackend>,
    selected_backend_index: usize,
}

impl Cocuyo {
    pub fn new(
        frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<FrameData>>>,
        recording_state: Arc<Mutex<RecordingState>>,
        start_recording_tx: std::sync::mpsc::Sender<((), GpuBackend)>,
        stop_flag: Arc<AtomicBool>,
        available_backends: Vec<GpuBackend>,
    ) -> (Self, Task<Message>) {
        let (id, open) = window::open(window::Settings {
            size: Size::new(800.0, 500.0),
            min_size: Some(Size::new(400.0, 300.0)),
            decorations: false,
            ..Default::default()
        });

        let _ = id;
        let app = Self {
            windows: BTreeMap::new(),
            frame_receiver,
            current_frame: None,
            recording_state,
            start_recording_tx,
            stop_flag,
            available_backends,
            selected_backend_index: 0,
        };

        (app, open.map(move |id| Message::WindowOpened(id, WindowKind::Main)))
    }

    pub fn title(&self, window_id: window::Id) -> String {
        match self.windows.get(&window_id) {
            Some(WindowKind::Main) => "Cocuyo".to_string(),
            Some(WindowKind::Settings) => "Cocuyo - Settings".to_string(),
            Some(WindowKind::Preview) => "Cocuyo - Preview".to_string(),
            None => String::new(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowOpened(id, kind) => {
                self.windows.insert(id, kind);
                Task::none()
            }
            Message::WindowClosed(id) => {
                let kind = self.windows.remove(&id);
                if kind == Some(WindowKind::Main) || self.windows.is_empty() {
                    iced::exit()
                } else {
                    Task::none()
                }
            }
            Message::DragWindow(id) => window::drag(id),
            Message::CloseWindow(id) => window::close(id),
            Message::MinimizeWindow(id) => window::minimize(id, true),
            Message::MaximizeWindow(id) => window::maximize(id, true),
            Message::OpenSettings => {
                if self.windows.values().any(|k| *k == WindowKind::Settings) {
                    return Task::none();
                }
                let (_id, open) = window::open(window::Settings {
                    size: Size::new(400.0, 300.0),
                    min_size: Some(Size::new(300.0, 200.0)),
                    decorations: false,
                    ..Default::default()
                });
                open.map(|id| Message::WindowOpened(id, WindowKind::Settings))
            }
            Message::OpenPreview => {
                if self.windows.values().any(|k| *k == WindowKind::Preview) {
                    return Task::none();
                }
                let (_id, open) = window::open(window::Settings {
                    size: Size::new(800.0, 600.0),
                    min_size: Some(Size::new(320.0, 240.0)),
                    decorations: false,
                    ..Default::default()
                });
                open.map(|id| Message::WindowOpened(id, WindowKind::Preview))
            }
            Message::StartRecording => {
                let backend = self
                    .available_backends
                    .get(self.selected_backend_index)
                    .cloned()
                    .unwrap_or(GpuBackend::Cpu);
                crate::vulkan_dmabuf::reset_dmabuf_import_failed();
                let _ = self.start_recording_tx.send(((), backend));
                Task::none()
            }
            Message::StopRecording => {
                self.stop_flag.store(true, Ordering::SeqCst);
                self.current_frame = None;
                // Drain any remaining frames from the channel to avoid stale data
                let mut receiver = self.frame_receiver.lock().unwrap();
                while receiver.try_recv().is_ok() {}
                Task::none()
            }
            Message::BackendSelected(idx) => {
                self.selected_backend_index = idx;
                Task::none()
            }
            Message::Tick => {
                let mut receiver = self.frame_receiver.lock().unwrap();
                while let Ok(frame) = receiver.try_recv() {
                    self.current_frame = Some(frame);
                }
                Task::none()
            }
        }
    }

    pub fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        let content = match self.windows.get(&window_id) {
            Some(WindowKind::Main) => {
                let state = self.recording_state.lock().unwrap().clone();
                let frame_info = self.current_frame.as_ref().map(|f| (f.width(), f.height()));
                crate::screen::main_window::view(window_id, &state, frame_info)
            }
            Some(WindowKind::Settings) => {
                let selected = self.available_backends.get(self.selected_backend_index);
                crate::screen::settings::view(window_id, &self.available_backends, selected)
            }
            Some(WindowKind::Preview) => {
                crate::screen::preview::view(window_id, self.current_frame.as_ref())
            }
            None => iced::widget::space().into(),
        };

        container(content)
            .width(Fill)
            .height(Fill)
            .padding(1)
            .style(crate::theme::window_border_container)
            .into()
    }

    pub fn theme(&self, _window_id: window::Id) -> Theme {
        crate::theme::create_theme()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick),
            window::close_events().map(Message::WindowClosed),
        ])
    }
}
