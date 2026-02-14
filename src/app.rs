use std::collections::{BTreeMap, HashSet};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Duration;

use drm_fourcc::DrmFourcc;
use iced::widget::container;
use iced::window;
use iced::{Fill, Size, Subscription, Task, Theme};
use tokio::sync::mpsc;
use crate::gst_pipeline::GpuBackend;
use crate::recording::{self, RecordingCommand, RecordingEvent};
use crate::screen::WindowKind;
use crate::widget::Element;

#[derive(Debug, Clone, PartialEq)]
pub enum RecordingState {
    Idle,
    Starting,
    Recording,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct BulbInfo {
    pub mac: String,
    pub ip: std::net::IpAddr,
    pub name: Option<String>,
}

impl std::fmt::Debug for FrameData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameData::DmaBuf { width, height, drm_format, .. } => {
                f.debug_struct("DmaBuf")
                    .field("width", width)
                    .field("height", height)
                    .field("drm_format", drm_format)
                    .finish()
            }
            FrameData::Cpu { width, height, .. } => {
                f.debug_struct("Cpu")
                    .field("width", width)
                    .field("height", height)
                    .finish()
            }
        }
    }
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
    RecordingEvent(RecordingEvent),
    ScanBulbs,
    BulbsDiscovered(Vec<BulbInfo>),
    ToggleBulb(String),
}

pub struct Cocuyo {
    windows: BTreeMap<window::Id, WindowKind>,
    current_frame: Option<Arc<FrameData>>,
    recording_state: RecordingState,
    is_recording: bool,
    session_id: u64,
    available_backends: Vec<GpuBackend>,
    selected_backend_index: usize,
    recording_cmd_tx: Option<mpsc::Sender<RecordingCommand>>,
    discovered_bulbs: Vec<BulbInfo>,
    selected_bulbs: HashSet<String>,
    is_scanning: bool,
}

impl Cocuyo {
    pub fn new(
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
            current_frame: None,
            recording_state: RecordingState::Idle,
            is_recording: false,
            session_id: 0,
            available_backends,
            selected_backend_index: 0,
            recording_cmd_tx: None,
            discovered_bulbs: Vec::new(),
            selected_bulbs: HashSet::new(),
            is_scanning: false,
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
                crate::vulkan_dmabuf::reset_dmabuf_import_failed();
                self.is_recording = true;
                self.session_id += 1;
                Task::none()
            }
            Message::StopRecording => {
                if let Some(cmd_tx) = self.recording_cmd_tx.take() {
                    let _ = cmd_tx.try_send(RecordingCommand::Stop);
                }
                self.current_frame = None;
                Task::none()
            }
            Message::BackendSelected(idx) => {
                self.selected_backend_index = idx;
                Task::none()
            }
            Message::ScanBulbs => {
                self.is_scanning = true;
                self.discovered_bulbs.clear();
                Task::perform(
                    async {
                        match wiz_lights_rs::discover_bulbs(Duration::from_secs(5)).await {
                            Ok(bulbs) => bulbs
                                .into_iter()
                                .map(|b| BulbInfo {
                                    ip: std::net::IpAddr::V4(b.ip),
                                    mac: b.mac.clone(),
                                    name: None,
                                })
                                .collect(),
                            Err(e) => {
                                tracing::error!(error = %e, "Bulb discovery failed");
                                Vec::new()
                            }
                        }
                    },
                    Message::BulbsDiscovered,
                )
            }
            Message::BulbsDiscovered(bulbs) => {
                self.is_scanning = false;
                self.discovered_bulbs = bulbs;
                Task::none()
            }
            Message::ToggleBulb(mac) => {
                if !self.selected_bulbs.remove(&mac) {
                    self.selected_bulbs.insert(mac);
                }
                Task::none()
            }
            Message::RecordingEvent(event) => {
                match event {
                    RecordingEvent::Ready(cmd_tx) => {
                        self.recording_cmd_tx = Some(cmd_tx);
                    }
                    RecordingEvent::StateChanged(state) => {
                        if state == RecordingState::Idle {
                            self.is_recording = false;
                            self.recording_cmd_tx = None;
                        }
                        self.recording_state = state;
                    }
                    RecordingEvent::Frame(frame) => {
                        self.current_frame = Some(frame);
                    }
                }
                Task::none()
            }
        }
    }

    pub fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        let content = match self.windows.get(&window_id) {
            Some(WindowKind::Main) => {
                crate::screen::main_window::view(
                    window_id,
                    &self.discovered_bulbs,
                    &self.selected_bulbs,
                    self.is_scanning,
                )
            }
            Some(WindowKind::Settings) => {
                let selected = self.available_backends.get(self.selected_backend_index);
                crate::screen::settings::view(window_id, &self.available_backends, selected)
            }
            Some(WindowKind::Preview) => {
                let frame_info = self.current_frame.as_ref().map(|f| (f.width(), f.height()));
                crate::screen::preview::view(
                    window_id,
                    self.current_frame.as_ref().map(|f| f.as_ref()),
                    &self.recording_state,
                    frame_info,
                )
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
        let mut subs = vec![window::close_events().map(Message::WindowClosed)];

        if self.is_recording {
            let backend = self
                .available_backends
                .get(self.selected_backend_index)
                .cloned()
                .unwrap_or(GpuBackend::Cpu);

            subs.push(
                Subscription::run_with(
                    (self.session_id, backend),
                    recording::recording_subscription,
                )
                .map(Message::RecordingEvent),
            );
        }

        Subscription::batch(subs)
    }
}
