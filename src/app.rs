use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::container;
use iced::window;
use iced::{Fill, Size, Subscription, Task, Theme};
use tokio::sync::mpsc;
use crate::bulb_setup::{BulbSetupMessage, BulbSetupState};
use crate::frame::FrameData;
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
pub enum Message {
    WindowOpened(window::Id, WindowKind),
    WindowClosed(window::Id),
    DragWindow(window::Id),
    CloseWindow(window::Id),
    MinimizeWindow(window::Id),
    MaximizeWindow(window::Id),
    OpenSettings,
    OpenPreview,
    OpenBulbSetup,
    StartRecording,
    StopRecording,
    BackendSelected(usize),
    RecordingEvent(RecordingEvent),
    BulbSetup(BulbSetupMessage),
    StartAmbient,
    StopAmbient,
    Noop,
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
    bulb_setup: BulbSetupState,
    is_ambient_active: bool,
    last_bulb_update: Option<Instant>,
}

impl Cocuyo {
    fn open_window(&self, kind: WindowKind, size: Size, min_size: Size) -> Task<Message> {
        if self.windows.values().any(|k| *k == kind) {
            return Task::none();
        }
        let (_id, open) = window::open(window::Settings {
            size,
            min_size: Some(min_size),
            decorations: false,
            ..Default::default()
        });
        open.map(move |id| Message::WindowOpened(id, kind))
    }

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
            bulb_setup: BulbSetupState::new(),
            is_ambient_active: false,
            last_bulb_update: None,
        };

        (app, open.map(move |id| Message::WindowOpened(id, WindowKind::Main)))
    }

    pub fn title(&self, window_id: window::Id) -> String {
        match self.windows.get(&window_id) {
            Some(WindowKind::Main) => "Cocuyo".to_string(),
            Some(WindowKind::Settings) => "Cocuyo - Settings".to_string(),
            Some(WindowKind::Preview) => "Cocuyo - Preview".to_string(),
            Some(WindowKind::BulbSetup) => "Cocuyo - Bulb Setup".to_string(),
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
                self.open_window(
                    WindowKind::Settings,
                    Size::new(400.0, 300.0),
                    Size::new(300.0, 200.0),
                )
            }
            Message::OpenPreview => {
                self.open_window(
                    WindowKind::Preview,
                    Size::new(800.0, 600.0),
                    Size::new(320.0, 240.0),
                )
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
            Message::OpenBulbSetup => {
                self.open_window(
                    WindowKind::BulbSetup,
                    Size::new(500.0, 400.0),
                    Size::new(350.0, 300.0),
                )
            }
            Message::BulbSetup(msg) => {
                if matches!(msg, BulbSetupMessage::Done) {
                    if let Some((&id, _)) = self.windows.iter().find(|(_, k)| **k == WindowKind::BulbSetup) {
                        return window::close(id);
                    }
                    return Task::none();
                }
                self.bulb_setup.update(msg).map(Message::BulbSetup)
            }
            Message::Noop => Task::none(),
            Message::StartAmbient => {
                if !self.bulb_setup.has_selected_bulbs() {
                    return Task::none();
                }
                self.is_ambient_active = true;
                self.last_bulb_update = None;
                if !self.is_recording {
                    crate::vulkan_dmabuf::reset_dmabuf_import_failed();
                    self.is_recording = true;
                    self.session_id += 1;
                }
                Task::none()
            }
            Message::StopAmbient => {
                self.is_ambient_active = false;
                self.last_bulb_update = None;
                if let Some(cmd_tx) = self.recording_cmd_tx.take() {
                    let _ = cmd_tx.try_send(RecordingCommand::Stop);
                }
                self.current_frame = None;
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
                            self.is_ambient_active = false;
                            self.recording_cmd_tx = None;
                        }
                        self.recording_state = state;
                    }
                    RecordingEvent::Frame(frame) => {
                        self.current_frame = Some(frame.clone());

                        if self.is_ambient_active {
                            let should_update = self
                                .last_bulb_update
                                .map(|t| t.elapsed() >= Duration::from_millis(150))
                                .unwrap_or(true);

                            if should_update {
                                self.last_bulb_update = Some(Instant::now());
                                let selected = self.bulb_setup.selected_bulbs_vec();
                                if let Some(targets) = crate::ambient::sample_frame_for_bulbs(
                                    &frame,
                                    &selected,
                                    self.bulb_setup.discovered_bulbs(),
                                ) {
                                    return Task::perform(
                                        crate::ambient::dispatch_bulb_colors(targets),
                                        |()| Message::Noop,
                                    );
                                }
                            }
                        }
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
                    self.is_ambient_active,
                    self.bulb_setup.has_selected_bulbs(),
                    self.bulb_setup.selected_bulbs().len(),
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
                    self.is_ambient_active,
                )
            }
            Some(WindowKind::BulbSetup) => {
                crate::screen::bulb_setup::view(window_id, &self.bulb_setup)
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
