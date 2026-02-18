use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ambient::SavedBulbState;
use crate::bulb_setup::{BulbSetupMessage, BulbSetupState};
use crate::frame::FrameData;
use crate::platform::linux::gst_pipeline::GpuBackend;
use crate::recording::{self, RecordingCommand, RecordingEvent};
use crate::region::Region;
use crate::screen::WindowKind;
use crate::screen::region_overlay::RegionMessage;
use crate::widget::Element;
use iced::widget::container;
use iced::window;
use iced::{Fill, Size, Subscription, Task, Theme};
use tokio::sync::mpsc;

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
    OpenBulbSetup,
    StartRecording,
    StopRecording,
    BackendSelected(usize),
    RecordingEvent(RecordingEvent),
    BulbSetup(BulbSetupMessage),
    StartAmbient,
    StopAmbient,
    BulbStatesSaved(Vec<SavedBulbState>),
    RegionUpdate(RegionMessage),
    ExitApp,
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
    saved_bulb_states: Option<Vec<SavedBulbState>>,
    regions: Vec<Region>,
    next_region_id: usize,
    selected_region: Option<usize>,
}

impl Cocuyo {
    pub fn new(available_backends: Vec<GpuBackend>) -> (Self, Task<Message>) {
        let (id, open) = window::open(window::Settings {
            size: Size::new(1200.0, 750.0),
            min_size: Some(Size::new(800.0, 500.0)),
            decorations: false,
            transparent: true,
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
            saved_bulb_states: None,
            regions: Vec::new(),
            next_region_id: 1,
            selected_region: None,
        };

        (
            app,
            open.map(move |id| Message::WindowOpened(id, WindowKind::Main)),
        )
    }

    pub fn title(&self, window_id: window::Id) -> String {
        match self.windows.get(&window_id) {
            Some(WindowKind::Main) => "Cocuyo".to_string(),
            Some(WindowKind::Settings) => "Cocuyo - Settings".to_string(),
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
                    // Stop recording if active
                    if self.is_ambient_active || self.is_recording {
                        if let Some(cmd_tx) = self.recording_cmd_tx.take() {
                            let _ = cmd_tx.try_send(RecordingCommand::Stop);
                        }
                    }
                    // Restore bulb states before exiting
                    if let Some(states) = self.saved_bulb_states.take() {
                        Task::perform(crate::ambient::restore_bulb_states(states), |()| {
                            Message::ExitApp
                        })
                    } else {
                        iced::exit()
                    }
                } else {
                    Task::none()
                }
            }
            Message::DragWindow(id) => window::drag(id),
            Message::CloseWindow(id) => window::close(id),
            Message::MinimizeWindow(id) => window::minimize(id, true),
            Message::MaximizeWindow(id) => window::maximize(id, true),
            Message::OpenSettings => self.open_window(
                WindowKind::Settings,
                Size::new(400.0, 300.0),
                Size::new(300.0, 200.0),
            ),
            Message::StartRecording => {
                crate::platform::linux::vulkan_dmabuf::reset_dmabuf_import_failed();
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
            Message::OpenBulbSetup => self.open_window(
                WindowKind::BulbSetup,
                Size::new(500.0, 400.0),
                Size::new(350.0, 300.0),
            ),
            Message::BulbSetup(msg) => {
                if matches!(msg, BulbSetupMessage::Done) {
                    self.sync_regions_to_bulbs();
                    if let Some((&id, _)) = self
                        .windows
                        .iter()
                        .find(|(_, k)| **k == WindowKind::BulbSetup)
                    {
                        return window::close(id);
                    }
                    return Task::none();
                }
                let is_toggle = matches!(msg, BulbSetupMessage::ToggleBulb(_));
                let task = self.bulb_setup.update(msg).map(Message::BulbSetup);
                if is_toggle {
                    self.sync_regions_to_bulbs();
                }
                task
            }
            Message::Noop => Task::none(),
            Message::ExitApp => iced::exit(),
            Message::StartAmbient => {
                if !self.bulb_setup.has_selected_bulbs() {
                    return Task::none();
                }
                // Phase 1: save bulb states before starting ambient
                let bulbs = self.bulb_setup.selected_bulb_infos();
                Task::perform(
                    crate::ambient::save_bulb_states(bulbs),
                    Message::BulbStatesSaved,
                )
            }
            Message::BulbStatesSaved(states) => {
                // Phase 2: states saved, now start ambient
                self.saved_bulb_states = if states.is_empty() {
                    None
                } else {
                    Some(states)
                };
                self.is_ambient_active = true;
                self.last_bulb_update = None;
                if !self.is_recording {
                    crate::platform::linux::vulkan_dmabuf::reset_dmabuf_import_failed();
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
                let restore_task = if let Some(states) = self.saved_bulb_states.take() {
                    Task::perform(crate::ambient::restore_bulb_states(states), |()| {
                        Message::Noop
                    })
                } else {
                    Task::none()
                };
                restore_task
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
                        self.current_frame = Some(frame);
                        let frame = self.current_frame.as_ref().unwrap();

                        // Update region sampled colors
                        for region in &mut self.regions {
                            region.sampled_color = frame.sample_region_average(
                                region.x,
                                region.y,
                                region.width,
                                region.height,
                            );
                        }

                        if self.is_ambient_active {
                            let should_update = self
                                .last_bulb_update
                                .map(|t| t.elapsed() >= Duration::from_millis(150))
                                .unwrap_or(true);

                            if should_update {
                                self.last_bulb_update = Some(Instant::now());

                                // Use region-based sampling when regions exist
                                if self.regions.is_empty() {
                                    tracing::debug!("No regions defined, skipping bulb update");
                                }

                                if let Some(targets) = crate::ambient::sample_frame_for_regions(
                                    frame,
                                    &self.regions,
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
            Message::RegionUpdate(msg) => {
                match msg {
                    RegionMessage::Updated(id, x, y, w, h) => {
                        if let Some(existing) = self.regions.iter_mut().find(|reg| reg.id == id) {
                            existing.x = x;
                            existing.y = y;
                            existing.width = w;
                            existing.height = h;
                        }
                    }
                    RegionMessage::Selected(id) => {
                        self.selected_region = id;
                    }
                }
                Task::none()
            }
        }
    }

    pub fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        let content = match self.windows.get(&window_id) {
            Some(WindowKind::Main) => {
                let frame_info = self.current_frame.as_ref().map(|f| (f.width(), f.height()));
                crate::screen::main_window::view(
                    window_id,
                    self.current_frame.as_ref().map(|f| f.as_ref()),
                    &self.recording_state,
                    frame_info,
                    self.is_ambient_active,
                    self.bulb_setup.has_selected_bulbs(),
                    self.bulb_setup.selected_bulbs().len(),
                    &self.regions,
                    self.selected_region,
                )
            }
            Some(WindowKind::Settings) => {
                let selected = self.available_backends.get(self.selected_backend_index);
                crate::screen::settings::view(window_id, &self.available_backends, selected)
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

    fn sync_regions_to_bulbs(&mut self) {
        let selected_macs = self.bulb_setup.selected_bulbs_vec();

        // Remove regions whose bulb_mac is no longer selected
        self.regions.retain(|r| {
            selected_macs.contains(&r.bulb_mac)
        });

        // Clear selected_region if it was removed
        if let Some(sel) = self.selected_region {
            if !self.regions.iter().any(|r| r.id == sel) {
                self.selected_region = None;
            }
        }

        // Add new regions for newly selected bulbs (not already covered)
        let num_total = selected_macs.len();
        for (i, mac) in selected_macs.iter().enumerate() {
            if self.regions.iter().any(|r| r.bulb_mac == *mac) {
                continue;
            }

            // Default layout: evenly spaced horizontally, vertically centered
            let (frame_w, frame_h) = self
                .current_frame
                .as_ref()
                .map(|f| (f.width() as f32, f.height() as f32))
                .unwrap_or((1920.0, 1080.0));

            let default_w = (frame_w / (num_total as f32 + 1.0)).min(frame_w * 0.3);
            let default_h = (frame_h * 0.4).min(frame_h * 0.6);
            let cx = frame_w * (i as f32 + 1.0) / (num_total as f32 + 1.0);
            let cy = frame_h / 2.0;

            let region = Region {
                id: self.next_region_id,
                x: cx - default_w / 2.0,
                y: cy,
                width: default_w,
                height: default_h,
                bulb_mac: mac.clone(),
                sampled_color: None,
            };
            self.next_region_id += 1;
            self.regions.push(region);
        }
    }

    fn open_window(&self, kind: WindowKind, size: Size, min_size: Size) -> Task<Message> {
        if self.windows.values().any(|k| *k == kind) {
            return Task::none();
        }
        let (_id, open) = window::open(window::Settings {
            size,
            min_size: Some(min_size),
            decorations: false,
            transparent: true,
            ..Default::default()
        });
        open.map(move |id| Message::WindowOpened(id, kind))
    }
}
