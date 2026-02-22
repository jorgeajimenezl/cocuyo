use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ambient::SavedBulbState;
use crate::config::AppConfig;
use crate::frame::FrameData;
#[cfg(target_os = "windows")]
use crate::platform::windows::capture_target::{CaptureTarget, PickerIntent};
use crate::recording::{self, RecordingCommand, RecordingEvent};
use crate::region::Region;
use crate::sampling::SamplingStrategy;
#[cfg(target_os = "windows")]
use crate::screen::capture_picker;
use crate::screen::settings;
use crate::screen::WindowKind;
use crate::screen::region_overlay::RegionMessage;
use crate::screen::bulb_setup;
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
    // Window lifecycle
    WindowOpened(window::Id, WindowKind),
    WindowClosed(window::Id),

    // Title bar actions (shared across all windows)
    DragWindow(window::Id),
    CloseWindow(window::Id),
    MinimizeWindow(window::Id),
    MaximizeWindow(window::Id),

    // Main window controls
    OpenSettings(window::Id),
    OpenBulbSetup(window::Id),
    StartRecording,
    StopRecording,
    StartAmbient,
    StopAmbient,
    BulbStatesSaved(Vec<SavedBulbState>),
    RecordingEvent(RecordingEvent),
    RegionUpdate(RegionMessage),
    RegionStrategyChanged(usize, SamplingStrategy),

    // Delegated screens
    Settings(settings::Message),
    BulbSetup(bulb_setup::Message),
    #[cfg(target_os = "windows")]
    CapturePicker(capture_picker::Message),

    ExitApp,
    Noop,
}

pub struct Cocuyo {
    windows: BTreeMap<window::Id, WindowKind>,
    current_frame: Option<Arc<FrameData>>,
    recording_state: RecordingState,
    is_recording: bool,
    session_id: u64,
    recording_cmd_tx: Option<mpsc::Sender<RecordingCommand>>,
    bulb_setup: bulb_setup::BulbSetupState,
    is_ambient_active: bool,
    last_bulb_update: Option<Instant>,
    saved_bulb_states: Option<Vec<SavedBulbState>>,
    regions: Vec<Region>,
    next_region_id: usize,
    selected_region: Option<usize>,
    config: AppConfig,

    // Screen state
    settings: settings::Settings,
    #[cfg(target_os = "windows")]
    capture_picker: Option<capture_picker::CapturePicker>,
    #[cfg(target_os = "windows")]
    capture_target: Option<CaptureTarget>,
}

impl Cocuyo {
    pub fn new(config: AppConfig) -> (Self, Task<Message>) {
        let settings = settings::Settings::new(&config);

        let (id, open) = window::open(window::Settings {
            size: Size::new(1200.0, 750.0),
            min_size: Some(Size::new(800.0, 500.0)),
            decorations: false,
            transparent: true,
            ..Default::default()
        });

        let _ = id;
        let saved_bulbs = config.saved_bulbs.clone();
        let selected_macs = config.selected_bulb_macs.iter().cloned().collect();
        let mut app = Self {
            windows: BTreeMap::new(),
            current_frame: None,
            recording_state: RecordingState::Idle,
            is_recording: false,
            session_id: 0,
            recording_cmd_tx: None,
            bulb_setup: bulb_setup::BulbSetupState::new(saved_bulbs, selected_macs),
            is_ambient_active: false,
            last_bulb_update: None,
            saved_bulb_states: None,
            regions: Vec::new(),
            next_region_id: 1,
            selected_region: None,
            settings,
            config,
            #[cfg(target_os = "windows")]
            capture_picker: None,
            #[cfg(target_os = "windows")]
            capture_target: None,
        };
        app.sync_regions_to_bulbs();

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
            #[cfg(target_os = "windows")]
            Some(WindowKind::CapturePicker) => "Cocuyo - Select Target".to_string(),
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
                #[cfg(target_os = "windows")]
                if kind == Some(WindowKind::CapturePicker) {
                    self.capture_picker = None;
                    return Task::none();
                }
                if kind == Some(WindowKind::Main) || self.windows.is_empty() {
                    if self.is_ambient_active || self.is_recording {
                        if let Some(cmd_tx) = self.recording_cmd_tx.take() {
                            let _ = cmd_tx.try_send(RecordingCommand::Stop);
                        }
                    }
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
            Message::OpenSettings(parent) => self.open_window(
                WindowKind::Settings,
                Size::new(500.0, 500.0),
                Size::new(300.0, 200.0),
                Some(parent),
            ),
            Message::StartRecording => {
                #[cfg(target_os = "linux")]
                {
                    crate::platform::linux::vulkan_dmabuf::reset_dmabuf_import_failed();
                    self.is_recording = true;
                    self.session_id += 1;
                }
                #[cfg(target_os = "windows")]
                {
                    return self.open_capture_picker(PickerIntent::StartRecording);
                }
                #[allow(unreachable_code)]
                Task::none()
            }
            Message::StopRecording => {
                if let Some(cmd_tx) = self.recording_cmd_tx.take() {
                    let _ = cmd_tx.try_send(RecordingCommand::Stop);
                }
                self.current_frame = None;
                Task::none()
            }
            Message::OpenBulbSetup(parent) => self.open_window(
                WindowKind::BulbSetup,
                Size::new(500.0, 400.0),
                Size::new(350.0, 300.0),
                Some(parent),
            ),
            Message::Settings(msg) => {
                let (task, event) = self.settings.update(msg);
                let task = task.map(Message::Settings);
                if let Some(event) = event {
                    let event_task = self.handle_settings_event(event);
                    Task::batch([task, event_task])
                } else {
                    task
                }
            }
            Message::BulbSetup(msg) => {
                let (task, event) = self.bulb_setup.update(msg);
                let task = task.map(Message::BulbSetup);
                if let Some(event) = event {
                    let event_task = self.handle_bulb_setup_event(event);
                    Task::batch([task, event_task])
                } else {
                    task
                }
            }
            #[cfg(target_os = "windows")]
            Message::CapturePicker(msg) => {
                let Some(picker) = self.capture_picker.as_mut() else {
                    return Task::none();
                };
                let (task, event) = picker.update(msg);
                let task = task.map(Message::CapturePicker);
                if let Some(event) = event {
                    let event_task = self.handle_capture_picker_event(event);
                    Task::batch([task, event_task])
                } else {
                    task
                }
            }
            Message::Noop => Task::none(),
            Message::ExitApp => iced::exit(),
            Message::StartAmbient => {
                if !self.bulb_setup.has_selected_bulbs() {
                    return Task::none();
                }
                #[cfg(target_os = "windows")]
                {
                    return self.open_capture_picker(PickerIntent::StartAmbient);
                }
                #[allow(unreachable_code)]
                {
                    let bulbs = self.bulb_setup.selected_bulb_infos();
                    Task::perform(
                        crate::ambient::save_bulb_states(bulbs),
                        Message::BulbStatesSaved,
                    )
                }
            }
            Message::BulbStatesSaved(states) => {
                self.saved_bulb_states = if states.is_empty() {
                    None
                } else {
                    Some(states)
                };
                self.is_ambient_active = true;
                self.last_bulb_update = None;
                if !self.is_recording {
                    #[cfg(target_os = "linux")]
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
                if let Some(states) = self.saved_bulb_states.take() {
                    Task::perform(crate::ambient::restore_bulb_states(states), |()| {
                        Message::Noop
                    })
                } else {
                    Task::none()
                }
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

                        if self.is_ambient_active {
                            let should_update = self
                                .last_bulb_update
                                .map(|t| t.elapsed() >= Duration::from_millis(150))
                                .unwrap_or(true);

                            if should_update {
                                self.last_bulb_update = Some(Instant::now());

                                if self.regions.is_empty() {
                                    return Task::none();
                                }

                                let sampling_frame = frame.convert_to_cpu();

                                if let Some(ref sf) = sampling_frame {
                                    for region in &mut self.regions {
                                        region.sampled_color = crate::sampling::sample_region(
                                            sf,
                                            region.x,
                                            region.y,
                                            region.width,
                                            region.height,
                                            region.strategy,
                                        );
                                    }

                                    if let Some(targets) =
                                        crate::ambient::sample_frame_for_regions(
                                            sf,
                                            &self.regions,
                                            self.bulb_setup.discovered_bulbs(),
                                        )
                                    {
                                        return Task::perform(
                                            crate::ambient::dispatch_bulb_colors(targets),
                                            |()| Message::Noop,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                Task::none()
            }
            Message::RegionStrategyChanged(id, strategy) => {
                if let Some(region) = self.regions.iter_mut().find(|r| r.id == id) {
                    region.strategy = strategy;
                }
                Task::none()
            }
            Message::RegionUpdate(msg) => {
                match msg {
                    RegionMessage::Updated(id, x, y, w, h) => {
                        if let Some(existing) =
                            self.regions.iter_mut().find(|reg| reg.id == id)
                        {
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
        use iced::widget::{column, rule};
        use crate::screen::title_bar;

        let title = match self.windows.get(&window_id) {
            Some(WindowKind::Main) => "Cocuyo",
            Some(WindowKind::Settings) => "Settings",
            Some(WindowKind::BulbSetup) => "Bulb Setup",
            #[cfg(target_os = "windows")]
            Some(WindowKind::CapturePicker) => "Select Capture Target",
            None => "",
        };

        let screen_content = match self.windows.get(&window_id) {
            Some(WindowKind::Main) => {
                let frame_info =
                    self.current_frame.as_ref().map(|f| (f.width(), f.height()));
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
            Some(WindowKind::Settings) => self
                .settings
                .view()
                .map(Message::Settings),
            Some(WindowKind::BulbSetup) => self
                .bulb_setup
                .view()
                .map(Message::BulbSetup),
            #[cfg(target_os = "windows")]
            Some(WindowKind::CapturePicker) => {
                if let Some(ref picker) = self.capture_picker {
                    picker.view().map(Message::CapturePicker)
                } else {
                    iced::widget::space().into()
                }
            }
            None => iced::widget::space().into(),
        };

        let content = column![
            title_bar::view(window_id, title),
            rule::horizontal(1).style(crate::theme::styled_rule),
            screen_content,
        ]
        .width(Fill)
        .height(Fill);

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
            subs.push(self.build_recording_subscription());
        }

        Subscription::batch(subs)
    }

    #[cfg(target_os = "linux")]
    fn build_recording_subscription(&self) -> Subscription<Message> {
        let backend = self.settings.selected_backend();

        Subscription::run_with(
            (self.session_id, backend),
            recording::recording_subscription,
        )
        .map(Message::RecordingEvent)
    }

    #[cfg(target_os = "windows")]
    fn build_recording_subscription(&self) -> Subscription<Message> {
        let target = self
            .capture_target
            .expect("capture_target must be set before recording");
        Subscription::run_with(
            (self.session_id, target),
            recording::recording_subscription,
        )
        .map(Message::RecordingEvent)
    }

    // --- Event handlers for delegated screens ---

    fn handle_settings_event(&mut self, event: settings::Event) -> Task<Message> {
        match event {
            #[cfg(target_os = "linux")]
            settings::Event::BackendChanged(config_key) => {
                self.config.preferred_backend = config_key;
                self.config.save();
                Task::none()
            }
            settings::Event::AdapterChanged(preferred) => {
                self.config.preferred_adapter = preferred;
                self.config.save();
                Task::none()
            }
        }
    }

    fn handle_bulb_setup_event(&mut self, event: bulb_setup::BulbSetupEvent) -> Task<Message> {
        match event {
            bulb_setup::BulbSetupEvent::Done => {
                self.sync_regions_to_bulbs();
                self.save_bulb_config();
                self.close_window_by_kind(WindowKind::BulbSetup)
            }
            bulb_setup::BulbSetupEvent::SelectionChanged => {
                self.sync_regions_to_bulbs();
                self.save_bulb_config();
                Task::none()
            }
            bulb_setup::BulbSetupEvent::BulbsDiscovered => {
                self.save_bulb_config();
                Task::none()
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn handle_capture_picker_event(&mut self, event: capture_picker::Event) -> Task<Message> {
        match event {
            capture_picker::Event::TargetSelected(target, intent) => {
                self.capture_target = Some(target);
                let close_task = self.close_window_by_kind(WindowKind::CapturePicker);
                self.capture_picker = None;

                match intent {
                    PickerIntent::StartRecording => {
                        self.is_recording = true;
                        self.session_id += 1;
                        close_task
                    }
                    PickerIntent::StartAmbient => {
                        let bulbs = self.bulb_setup.selected_bulb_infos();
                        Task::batch([
                            close_task,
                            Task::perform(
                                crate::ambient::save_bulb_states(bulbs),
                                Message::BulbStatesSaved,
                            ),
                        ])
                    }
                }
            }
            capture_picker::Event::Cancelled => {
                self.capture_picker = None;
                self.close_window_by_kind(WindowKind::CapturePicker)
            }
        }
    }

    // --- Window helpers ---

    fn find_window_id(&self, kind: WindowKind) -> Option<window::Id> {
        self.windows
            .iter()
            .find(|(_, k)| **k == kind)
            .map(|(&id, _)| id)
    }

    fn close_window_by_kind(&self, kind: WindowKind) -> Task<Message> {
        if let Some(id) = self.find_window_id(kind) {
            window::close(id)
        } else {
            Task::none()
        }
    }

    // --- Shared helpers ---

    fn sync_regions_to_bulbs(&mut self) {
        let selected_macs = self.bulb_setup.selected_bulbs_vec();

        self.regions.retain(|r| selected_macs.contains(&r.bulb_mac));

        if let Some(sel) = self.selected_region {
            if !self.regions.iter().any(|r| r.id == sel) {
                self.selected_region = None;
            }
        }

        let num_total = selected_macs.len();
        for (i, mac) in selected_macs.iter().enumerate() {
            if self.regions.iter().any(|r| r.bulb_mac == *mac) {
                continue;
            }

            let (frame_w, frame_h) = self
                .current_frame
                .as_ref()
                .map(|f| (f.width() as f32, f.height() as f32))
                .unwrap_or((1920.0, 1080.0));

            let default_w = (frame_w / (num_total as f32 + 1.0)).min(frame_w * 0.3);
            let default_h = frame_h * 0.4;
            let cx = frame_w * (i as f32 + 1.0) / (num_total as f32 + 1.0);
            let cy = frame_h / 2.0;

            let region = Region {
                id: self.next_region_id,
                x: cx - default_w / 2.0,
                y: (cy - default_h / 2.0).clamp(0.0, frame_h - default_h),
                width: default_w,
                height: default_h,
                bulb_mac: mac.clone(),
                sampled_color: None,
                strategy: SamplingStrategy::default(),
            };
            self.next_region_id += 1;
            self.regions.push(region);
        }
    }

    fn open_window(
        &self,
        kind: WindowKind,
        size: Size,
        min_size: Size,
        parent: Option<window::Id>,
    ) -> Task<Message> {
        if self.windows.values().any(|k| *k == kind) {
            return Task::none();
        }
        let (_id, open) = window::open(window::Settings {
            size,
            min_size: Some(min_size),
            decorations: false,
            transparent: true,
            parent,
            ..Default::default()
        });
        open.map(move |id| Message::WindowOpened(id, kind))
    }

    #[cfg(target_os = "windows")]
    fn open_capture_picker(&mut self, intent: PickerIntent) -> Task<Message> {
        self.capture_picker = Some(capture_picker::CapturePicker::new(intent));

        let parent = self.find_window_id(WindowKind::Main);

        self.open_window(
            WindowKind::CapturePicker,
            Size::new(500.0, 500.0),
            Size::new(350.0, 300.0),
            parent,
        )
    }

    fn save_bulb_config(&mut self) {
        self.config.saved_bulbs = self.bulb_setup.discovered_bulbs().to_vec();
        self.config.selected_bulb_macs = self.bulb_setup.selected_bulbs_vec();
        self.config.save();
    }
}
