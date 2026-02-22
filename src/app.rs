use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::{self, GpuAdapterSelection};
use crate::ambient::SavedBulbState;
use crate::bulb_setup::{BulbSetupMessage, BulbSetupState};
use crate::config::AppConfig;
use crate::frame::FrameData;
#[cfg(target_os = "linux")]
use crate::platform::linux::gst_pipeline::{self, GpuBackend};
#[cfg(target_os = "windows")]
use crate::platform::windows::capture_target::{CaptureTarget, PickerIntent, PickerTab};
use crate::recording::{self, RecordingCommand, RecordingEvent};
use crate::region::Region;
use crate::sampling::SamplingStrategy;
use crate::screen::WindowKind;
use crate::screen::region_overlay::RegionMessage;
use crate::widget::Element;
use iced::widget::container;
use iced::window;
use iced::{Fill, Size, Subscription, Task, Theme};
use tokio::sync::mpsc;
use tracing::info;

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
    OpenSettings(window::Id),
    OpenBulbSetup(window::Id),
    StartRecording,
    StopRecording,
    #[cfg(target_os = "linux")]
    BackendSelected(usize),
    AdapterSelected(GpuAdapterSelection),
    RecordingEvent(RecordingEvent),
    BulbSetup(BulbSetupMessage),
    StartAmbient,
    StopAmbient,
    BulbStatesSaved(Vec<SavedBulbState>),
    RegionUpdate(RegionMessage),
    RegionStrategyChanged(usize, SamplingStrategy),
    #[cfg(target_os = "windows")]
    PickerSelectTarget(CaptureTarget),
    #[cfg(target_os = "windows")]
    PickerSwitchTab(PickerTab),
    #[cfg(target_os = "windows")]
    PickerConfirm,
    #[cfg(target_os = "windows")]
    PickerCancel,
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
    bulb_setup: BulbSetupState,
    is_ambient_active: bool,
    last_bulb_update: Option<Instant>,
    saved_bulb_states: Option<Vec<SavedBulbState>>,
    regions: Vec<Region>,
    next_region_id: usize,
    selected_region: Option<usize>,

    // Backend and adapter selection
    #[cfg(target_os = "linux")]
    available_backends: Vec<GpuBackend>,
    #[cfg(target_os = "linux")]
    selected_backend_index: usize,

    // Adapter used for displaying recording and
    // sampling frames (if applicable)
    available_adapters: Vec<String>,
    selected_adapter: GpuAdapterSelection,
    config: AppConfig,

    // Windows capture picker state
    #[cfg(target_os = "windows")]
    capture_picker_monitors: Vec<windows_capture::monitor::Monitor>,
    #[cfg(target_os = "windows")]
    capture_picker_windows: Vec<windows_capture::window::Window>,
    #[cfg(target_os = "windows")]
    capture_picker_selected: Option<CaptureTarget>,
    #[cfg(target_os = "windows")]
    capture_picker_tab: PickerTab,
    #[cfg(target_os = "windows")]
    capture_target: Option<CaptureTarget>,
    #[cfg(target_os = "windows")]
    pending_picker_intent: Option<PickerIntent>,
}

impl Cocuyo {
    pub fn new(config: AppConfig) -> (Self, Task<Message>) {
        #[cfg(target_os = "linux")]
        let (available_backends, selected_backend_index) = {
            let detected_backends = gst_pipeline::detect_available_backends();
            info!(backends = ?detected_backends, "Detected GPU backends");

            let mut available_backends = Vec::with_capacity(detected_backends.len() + 1);
            available_backends.push(GpuBackend::Auto);
            available_backends.extend(detected_backends);

            let selected_backend_index = config
                .preferred_backend
                .as_deref()
                .and_then(|key| {
                    available_backends
                        .iter()
                        .position(|b| b.config_key() == key)
                })
                .unwrap_or(0);

            (available_backends, selected_backend_index)
        };

        let available_adapters = adapters::enumerate_adapters();
        let selected_adapter =
            adapters::resolve_selection(config.preferred_adapter.as_deref(), &available_adapters);

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
            config,
            windows: BTreeMap::new(),
            current_frame: None,
            recording_state: RecordingState::Idle,
            is_recording: false,
            session_id: 0,
            #[cfg(target_os = "linux")]
            available_backends,
            #[cfg(target_os = "linux")]
            selected_backend_index,
            recording_cmd_tx: None,
            bulb_setup: BulbSetupState::new(saved_bulbs, selected_macs),
            is_ambient_active: false,
            last_bulb_update: None,
            saved_bulb_states: None,
            regions: Vec::new(),
            next_region_id: 1,
            selected_region: None,
            available_adapters,
            selected_adapter,
            #[cfg(target_os = "windows")]
            capture_picker_monitors: Vec::new(),
            #[cfg(target_os = "windows")]
            capture_picker_windows: Vec::new(),
            #[cfg(target_os = "windows")]
            capture_picker_selected: None,
            #[cfg(target_os = "windows")]
            capture_picker_tab: PickerTab::Screens,
            #[cfg(target_os = "windows")]
            capture_target: None,
            #[cfg(target_os = "windows")]
            pending_picker_intent: None,
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
                    self.pending_picker_intent = None;
                    self.capture_picker_selected = None;
                    return Task::none();
                }
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
            #[cfg(target_os = "linux")]
            Message::BackendSelected(idx) => {
                self.selected_backend_index = idx;
                if let Some(backend) = self.available_backends.get(idx) {
                    self.config.preferred_backend = Some(backend.config_key());
                    self.config.save();
                }
                Task::none()
            }
            Message::AdapterSelected(selection) => {
                self.selected_adapter = selection.clone();
                let preferred = match &selection {
                    GpuAdapterSelection::Auto => None,
                    GpuAdapterSelection::Named(name) => Some(name.clone()),
                };
                self.config.preferred_adapter = preferred;
                self.config.save();
                Task::none()
            }
            Message::OpenBulbSetup(parent) => self.open_window(
                WindowKind::BulbSetup,
                Size::new(500.0, 400.0),
                Size::new(350.0, 300.0),
                Some(parent),
            ),
            Message::BulbSetup(msg) => {
                if matches!(msg, BulbSetupMessage::Done) {
                    self.sync_regions_to_bulbs();
                    self.save_bulb_config();
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
                let is_discovered = matches!(msg, BulbSetupMessage::BulbsDiscovered(_));
                let task = self.bulb_setup.update(msg).map(Message::BulbSetup);
                if is_toggle {
                    self.sync_regions_to_bulbs();
                    self.save_bulb_config();
                }
                if is_discovered {
                    self.save_bulb_config();
                }
                task
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
                // Phase 1: save bulb states before starting ambient
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
                // Phase 2: states saved, now start ambient
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

                        if self.is_ambient_active {
                            let should_update = self
                                .last_bulb_update
                                .map(|t| t.elapsed() >= Duration::from_millis(150))
                                .unwrap_or(true);

                            if should_update {
                                self.last_bulb_update = Some(Instant::now());

                                // Use region-based sampling when regions exist
                                if self.regions.is_empty() {
                                    return Task::none();
                                }

                                // Read pixel data on demand — only here, every ~150ms
                                // NOTE: this data can be stale maybe because the recording thread may
                                // have already moved on to a newer frame, but that's ok since ambient
                                // lighting doesn't need to be perfectly in sync
                                let sampling_frame = frame.convert_to_cpu();

                                if let Some(ref sf) = sampling_frame {
                                    // Update region sampled colors
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

                                    if let Some(targets) = crate::ambient::sample_frame_for_regions(
                                        sf,
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
                }
                Task::none()
            }
            Message::RegionStrategyChanged(id, strategy) => {
                if let Some(region) = self.regions.iter_mut().find(|r| r.id == id) {
                    region.strategy = strategy;
                }
                Task::none()
            }
            #[cfg(target_os = "windows")]
            Message::PickerSelectTarget(target) => {
                self.capture_picker_selected = Some(target);
                Task::none()
            }
            #[cfg(target_os = "windows")]
            Message::PickerSwitchTab(tab) => {
                self.capture_picker_tab = tab;
                Task::none()
            }
            #[cfg(target_os = "windows")]
            Message::PickerConfirm => {
                let target = match self.capture_picker_selected.take() {
                    Some(t) => t,
                    None => return Task::none(),
                };
                self.capture_target = Some(target);

                // Close the picker window
                let close_task = if let Some((&id, _)) = self
                    .windows
                    .iter()
                    .find(|(_, k)| **k == WindowKind::CapturePicker)
                {
                    window::close(id)
                } else {
                    Task::none()
                };

                match self.pending_picker_intent.take() {
                    Some(PickerIntent::StartRecording) => {
                        self.is_recording = true;
                        self.session_id += 1;
                        close_task
                    }
                    Some(PickerIntent::StartAmbient) => {
                        // Trigger the ambient flow (save bulb states first)
                        let bulbs = self.bulb_setup.selected_bulb_infos();
                        Task::batch([
                            close_task,
                            Task::perform(
                                crate::ambient::save_bulb_states(bulbs),
                                Message::BulbStatesSaved,
                            ),
                        ])
                    }
                    None => close_task,
                }
            }
            #[cfg(target_os = "windows")]
            Message::PickerCancel => {
                self.pending_picker_intent = None;
                self.capture_picker_selected = None;
                if let Some((&id, _)) = self
                    .windows
                    .iter()
                    .find(|(_, k)| **k == WindowKind::CapturePicker)
                {
                    window::close(id)
                } else {
                    Task::none()
                }
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
                #[cfg(target_os = "linux")]
                {
                    let selected = self.available_backends.get(self.selected_backend_index);
                    crate::screen::settings::view(
                        window_id,
                        &self.available_backends,
                        selected,
                        &self.available_adapters,
                        &self.selected_adapter,
                        self.config.preferred_adapter.as_deref(),
                    )
                }
                #[cfg(not(target_os = "linux"))]
                {
                    crate::screen::settings::view(
                        window_id,
                        &self.available_adapters,
                        &self.selected_adapter,
                        self.config.preferred_adapter.as_deref(),
                    )
                }
            }
            Some(WindowKind::BulbSetup) => {
                crate::screen::bulb_setup::view(window_id, &self.bulb_setup)
            }
            #[cfg(target_os = "windows")]
            Some(WindowKind::CapturePicker) => {
                crate::screen::capture_picker::view(
                    window_id,
                    &self.capture_picker_monitors,
                    &self.capture_picker_windows,
                    self.capture_picker_selected.as_ref(),
                    self.capture_picker_tab,
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
            subs.push(self.build_recording_subscription());
        }

        Subscription::batch(subs)
    }

    #[cfg(target_os = "linux")]
    fn build_recording_subscription(&self) -> Subscription<Message> {
        let backend = self
            .available_backends
            .get(self.selected_backend_index)
            .cloned()
            .unwrap_or(GpuBackend::Cpu);

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

    fn sync_regions_to_bulbs(&mut self) {
        let selected_macs = self.bulb_setup.selected_bulbs_vec();

        // Remove regions whose bulb_mac is no longer selected
        self.regions.retain(|r| selected_macs.contains(&r.bulb_mac));

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
        use windows_capture::monitor::Monitor;
        use windows_capture::window::Window;

        self.capture_picker_monitors = Monitor::enumerate().unwrap_or_default();
        self.capture_picker_windows = Window::enumerate()
            .unwrap_or_default()
            .into_iter()
            .filter(|w| w.title().map(|t| !t.is_empty()).unwrap_or(false))
            .collect();
        self.capture_picker_selected = None;
        self.capture_picker_tab = PickerTab::Screens;
        self.pending_picker_intent = Some(intent);

        let parent = self
            .windows
            .iter()
            .find(|(_, k)| **k == WindowKind::Main)
            .map(|(&id, _)| id);

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
