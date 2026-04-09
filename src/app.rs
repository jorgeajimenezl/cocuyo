use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::container;
use iced::window;

use iced::{Fill, Size, Subscription, Task, Theme};
use tokio::sync::mpsc;

use cocuyo_core::frame::FrameData;
use cocuyo_core::recording::{RecordingCommand, RecordingEvent, RecordingState};
use cocuyo_sampling::BoxedStrategy;
use cocuyo_sampling::Region;

use crate::ambient::SavedBulbState;
use crate::config::AppConfig;
use crate::screen::WindowKind;
use crate::screen::bulb_setup;
use crate::screen::settings;
use crate::widget::Element;
use crate::widget::region_overlay::RegionMessage;

const MAIN_WINDOW_SIZE: Size = Size::new(1200.0, 750.0);
const MAIN_WINDOW_MIN: Size = Size::new(800.0, 500.0);
const SETTINGS_WINDOW_SIZE: Size = Size::new(500.0, 700.0);
const SETTINGS_WINDOW_MIN: Size = Size::new(300.0, 200.0);
const BULB_SETUP_WINDOW_SIZE: Size = Size::new(500.0, 400.0);
const BULB_SETUP_WINDOW_MIN: Size = Size::new(350.0, 300.0);
const PROFILE_DIALOG_SIZE: Size = Size::new(450.0, 400.0);
const PROFILE_DIALOG_MIN: Size = Size::new(350.0, 300.0);
#[cfg(target_os = "windows")]
const PICKER_WINDOW_SIZE: Size = Size::new(500.0, 500.0);
#[cfg(target_os = "windows")]
const PICKER_WINDOW_MIN: Size = Size::new(350.0, 300.0);
const DEFAULT_FRAME_SIZE: (f32, f32) = (1920.0, 1080.0);

#[cfg(target_os = "windows")]
use {
    crate::screen::capture_picker,
    cocuyo_platform_windows::capture_target::{CaptureTarget, PickerIntent},
    iced::window::settings::{PlatformSpecific, platform::CornerPreference},
};

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
    #[cfg(target_os = "windows")]
    OpenCapturePicker(Option<window::Id>, PickerIntent),
    StartRecording,
    StopRecording,
    StartAmbient,
    StopAmbient,
    BulbStatesSaved(Vec<SavedBulbState>),
    RecordingEvent(RecordingEvent),
    RegionUpdate(RegionMessage),
    RegionStrategyChanged(usize, BoxedStrategy),

    // Profiles
    OpenProfileDialog(window::Id),
    ProfileDialog(crate::screen::profile_dialog::Message),
    LoadProfile(String),

    // Delegated screens
    Settings(settings::Message),
    BulbSetup(bulb_setup::Message),
    #[cfg(target_os = "windows")]
    CapturePicker(capture_picker::Message),

    GpuSamplingComplete(cocuyo_sampling::gpu::SamplingResult),

    BulbDispatchComplete(f64),

    #[cfg_attr(not(feature = "tray"), allow(dead_code))]
    TrayEvent(crate::tray::TrayAction),

    ExitApp,
    Noop,
}

pub struct Cocuyo {
    windows: BTreeMap<window::Id, WindowKind>,
    config: AppConfig,

    // Recording state
    current_frame: Option<Arc<FrameData>>,
    last_frame_size: Option<(u32, u32)>,
    recording_state: RecordingState,
    is_recording: bool,
    session_id: u64,
    recording_fps_limit: u32,
    recording_resolution_scale: u32,
    recording_cmd_tx: Option<mpsc::Sender<RecordingCommand>>,
    bulb_setup: bulb_setup::BulbSetupState,
    is_ambient_active: bool,
    last_bulb_update: Option<Instant>,
    last_sent_colors: HashMap<String, (crate::ambient::BulbColor, u8)>,
    saved_bulb_states: Option<Vec<SavedBulbState>>,
    regions: Vec<Region>,
    next_region_id: usize,
    selected_region: Option<usize>,

    // GPU sampling worker
    sampling_worker: Option<cocuyo_sampling::gpu::SamplingWorker>,

    // Color smoother for transitions
    color_smoother: crate::ambient::ColorSmoother,

    // Performance stats
    perf_stats: crate::perf_stats::PerfStats,
    config_dirty: bool,

    window_icon: Option<window::Icon>,
    tray: &'static crate::tray::TrayState,
    tray_hide_requested: bool,

    // Profiles
    active_profile_name: Option<String>,
    profile_dialog: Option<crate::screen::profile_dialog::ProfileDialog>,

    // Screen state
    settings: settings::Settings,
    #[cfg(target_os = "windows")]
    capture_picker: Option<capture_picker::CapturePicker>,
    #[cfg(target_os = "windows")]
    capture_target: Option<CaptureTarget>,
}

impl Cocuyo {
    pub fn new(config: AppConfig, tray: &'static crate::tray::TrayState) -> (Self, Task<Message>) {
        let mut app = Self {
            windows: BTreeMap::new(),
            current_frame: None,
            last_frame_size: None,
            recording_state: RecordingState::Idle,
            is_recording: false,
            session_id: 0,
            recording_fps_limit: 0,
            recording_resolution_scale: 100,
            recording_cmd_tx: None,
            bulb_setup: bulb_setup::BulbSetupState::new(&config),
            is_ambient_active: false,
            last_bulb_update: None,
            last_sent_colors: HashMap::new(),
            saved_bulb_states: None,
            regions: Vec::new(),
            next_region_id: 1,
            selected_region: None,
            active_profile_name: None,
            profile_dialog: None,
            sampling_worker: None,
            color_smoother: crate::ambient::ColorSmoother::new(),
            perf_stats: crate::perf_stats::PerfStats::new(),
            config_dirty: false,
            window_icon: window::icon::from_rgba(
                include_bytes!(concat!(env!("OUT_DIR"), "/icon-window-256.rgba")).to_vec(),
                256,
                256,
            )
            .ok(),
            tray,
            tray_hide_requested: false,
            settings: settings::Settings::new(&config),
            config,
            #[cfg(target_os = "windows")]
            capture_picker: None,
            #[cfg(target_os = "windows")]
            capture_target: None,
        };
        app.sync_regions_to_bulbs();

        let task = app.open_window(WindowKind::Main, MAIN_WINDOW_SIZE, MAIN_WINDOW_MIN, None);

        (app, task)
    }

    pub fn title(&self, window_id: window::Id) -> String {
        match self.windows.get(&window_id) {
            Some(WindowKind::Main) => "Cocuyo".to_string(),
            Some(WindowKind::Settings) => "Cocuyo - Settings".to_string(),
            Some(WindowKind::BulbSetup) => "Cocuyo - Bulb Setup".to_string(),
            Some(WindowKind::ProfileDialog) => "Cocuyo - Profiles".to_string(),
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
                if kind == Some(WindowKind::ProfileDialog) {
                    self.profile_dialog = None;
                }
                if kind == Some(WindowKind::Settings)
                    || kind == Some(WindowKind::BulbSetup)
                    || kind == Some(WindowKind::ProfileDialog)
                {
                    self.flush_config();
                }
                if kind == Some(WindowKind::Main) {
                    #[cfg(feature = "tray")]
                    if self.config.minimize_to_tray || self.tray_hide_requested {
                        self.tray_hide_requested = false;
                        // Hide to tray — don't stop recording/ambient, don't exit
                        self.tray.update_menu_text(false, self.is_ambient_active);
                        return Task::none();
                    }
                    return self.graceful_shutdown();
                }
                Task::none()
            }
            Message::DragWindow(id) => window::drag(id),
            Message::CloseWindow(id) => window::close(id),
            Message::MinimizeWindow(id) => window::minimize(id, true),
            Message::MaximizeWindow(id) => window::maximize(id, true),
            Message::OpenSettings(parent) => self.open_window(
                WindowKind::Settings,
                SETTINGS_WINDOW_SIZE,
                SETTINGS_WINDOW_MIN,
                Some(parent),
            ),
            #[cfg(target_os = "windows")]
            Message::OpenCapturePicker(parent, intent) => {
                self.capture_picker = Some(capture_picker::CapturePicker::new(intent));
                self.open_window(
                    WindowKind::CapturePicker,
                    PICKER_WINDOW_SIZE,
                    PICKER_WINDOW_MIN,
                    parent,
                )
            }
            Message::StartRecording => {
                #[cfg(target_os = "linux")]
                {
                    cocuyo_platform_linux::vulkan_dmabuf::reset_dmabuf_import_failed();
                    self.begin_recording();
                    Task::none()
                }
                #[cfg(target_os = "macos")]
                {
                    cocuyo_platform_macos::metal_import::reset_iosurface_import_failed();
                    self.begin_recording();
                    Task::none()
                }
                #[cfg(target_os = "windows")]
                {
                    let parent = self.find_window_id(WindowKind::Main);
                    Task::done(Message::OpenCapturePicker(
                        parent,
                        PickerIntent::StartRecording,
                    ))
                }
            }
            Message::StopRecording => {
                self.perf_stats.reset();
                if let Some(cmd_tx) = self.recording_cmd_tx.take() {
                    let _ = cmd_tx.try_send(RecordingCommand::Stop);
                }
                self.current_frame = None;
                Task::none()
            }
            Message::OpenBulbSetup(parent) => self.open_window(
                WindowKind::BulbSetup,
                BULB_SETUP_WINDOW_SIZE,
                BULB_SETUP_WINDOW_MIN,
                Some(parent),
            ),
            Message::OpenProfileDialog(parent) => {
                if self.find_window_id(WindowKind::ProfileDialog).is_none() {
                    let can_save = self.last_frame_size.is_some();
                    self.profile_dialog = Some(crate::screen::profile_dialog::ProfileDialog::new(
                        &self.config.profiles,
                        self.active_profile_name.as_deref(),
                        can_save,
                    ));
                }
                self.open_window(
                    WindowKind::ProfileDialog,
                    PROFILE_DIALOG_SIZE,
                    PROFILE_DIALOG_MIN,
                    Some(parent),
                )
            }
            Message::ProfileDialog(msg) => {
                let Some(dialog) = self.profile_dialog.as_mut() else {
                    return Task::none();
                };
                let (task, event) = dialog.update(msg);
                let task = task.map(Message::ProfileDialog);
                if let Some(event) = event {
                    let event_task = self.handle_profile_dialog_event(event);
                    Task::batch([task, event_task])
                } else {
                    task
                }
            }
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
            Message::TrayEvent(action) => {
                use crate::tray::TrayAction;
                match action {
                    TrayAction::ToggleWindow => {
                        if let Some(id) = self.find_window_id(WindowKind::Main) {
                            self.tray_hide_requested = true;
                            self.tray.update_menu_text(false, self.is_ambient_active);
                            window::close(id)
                        } else {
                            self.tray.update_menu_text(true, self.is_ambient_active);
                            self.open_window(
                                WindowKind::Main,
                                MAIN_WINDOW_SIZE,
                                MAIN_WINDOW_MIN,
                                None,
                            )
                        }
                    }
                    TrayAction::ToggleAmbient => {
                        if self.is_ambient_active {
                            Task::done(Message::StopAmbient)
                        } else {
                            Task::done(Message::StartAmbient)
                        }
                    }
                    TrayAction::Exit => self.graceful_shutdown(),
                }
            }
            Message::Noop => Task::none(),
            Message::ExitApp => {
                self.flush_config();
                iced::exit()
            }
            Message::StartAmbient => {
                if !self.bulb_setup.has_selected_bulbs() {
                    return Task::none();
                }
                #[cfg(target_os = "windows")]
                {
                    let parent = self.find_window_id(WindowKind::Main);
                    Task::done(Message::OpenCapturePicker(
                        parent,
                        PickerIntent::StartAmbient,
                    ))
                }
                #[cfg(not(target_os = "windows"))]
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
                self.last_sent_colors.clear();
                self.color_smoother.clear();
                self.tray
                    .update_menu_text(self.find_window_id(WindowKind::Main).is_some(), true);

                // Lazily spawn GPU sampling worker when ambient starts
                if self.sampling_worker.is_none()
                    && !self.config.force_cpu_sampling
                    && let Some((device, queue)) = crate::gpu_context::get_gpu_context()
                {
                    self.sampling_worker = Some(cocuyo_sampling::gpu::SamplingWorker::spawn(
                        device.clone(),
                        queue.clone(),
                    ));
                }
                if !self.is_recording {
                    #[cfg(target_os = "linux")]
                    cocuyo_platform_linux::vulkan_dmabuf::reset_dmabuf_import_failed();
                    #[cfg(target_os = "windows")]
                    cocuyo_platform_windows::dx12_import::reset_d3d_shared_import_failed();
                    self.begin_recording();
                }
                Task::none()
            }
            Message::StopAmbient => {
                self.perf_stats.reset();
                self.is_ambient_active = false;
                self.last_bulb_update = None;
                self.last_sent_colors.clear();
                self.color_smoother.clear();
                self.sampling_worker = None;
                self.tray
                    .update_menu_text(self.find_window_id(WindowKind::Main).is_some(), false);
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
                        if matches!(state, RecordingState::Idle | RecordingState::Error(_)) {
                            self.perf_stats.reset();
                            self.is_recording = false;
                            self.is_ambient_active = false;
                            self.last_sent_colors.clear();
                            self.color_smoother.clear();
                            self.recording_cmd_tx = None;
                            self.current_frame = None;
                            self.tray.update_menu_text(
                                self.find_window_id(WindowKind::Main).is_some(),
                                false,
                            );
                        }
                        self.recording_state = state;
                    }
                    RecordingEvent::Frame(frame) => {
                        // Double check
                        if !self.is_recording {
                            return Task::none();
                        }

                        self.perf_stats.record_frame_arrival();
                        self.last_frame_size = Some((frame.width(), frame.height()));
                        self.current_frame = Some(frame);
                        let frame = self.current_frame.as_ref().unwrap();

                        if self.is_ambient_active {
                            let should_update = self
                                .last_bulb_update
                                .map(|t| {
                                    t.elapsed()
                                        >= Duration::from_millis(
                                            self.config.bulb_update_interval_ms,
                                        )
                                })
                                .unwrap_or(true);

                            if should_update {
                                if self.regions.is_empty() {
                                    return Task::none();
                                }

                                // Try GPU sampling (async, off main thread)
                                // Note: last_bulb_update is set in GpuSamplingComplete,
                                // not here, to avoid throttle drift when GPU is slow.
                                if let Some(ref worker) = self.sampling_worker {
                                    use cocuyo_sampling::gpu::{RegionParams, SendResult};
                                    if worker.is_idle() {
                                        let params: Vec<RegionParams> = self
                                            .regions
                                            .iter()
                                            .map(|r| RegionParams {
                                                region_id: r.id,
                                                x: r.x,
                                                y: r.y,
                                                width: r.width,
                                                height: r.height,
                                                strategy: r.strategy.clone(),
                                            })
                                            .collect();

                                        match worker.try_send(
                                            frame.clone(),
                                            params,
                                            Message::GpuSamplingComplete,
                                        ) {
                                            SendResult::Sent(task) => return task,
                                            SendResult::Busy => {} // race: became busy
                                            SendResult::Dead => {
                                                tracing::warn!(
                                                    "GPU sampling worker died, falling back to CPU"
                                                );
                                                self.sampling_worker = None;
                                            }
                                        }
                                    }
                                }

                                // CPU fallback (fast, stays on main thread)
                                if self.sampling_worker.is_none() {
                                    self.perf_stats.mark_sampling_start();
                                    self.last_bulb_update = Some(Instant::now());
                                    let sampling_frame = frame.convert_to_cpu();
                                    if let Some(ref sf) = sampling_frame {
                                        for region in &mut self.regions {
                                            region.sampled_color = cocuyo_sampling::sample_region(
                                                sf,
                                                region.x,
                                                region.y,
                                                region.width,
                                                region.height,
                                                &region.strategy,
                                            );
                                        }
                                    }
                                    self.perf_stats.record_sampling_complete();

                                    return self.dispatch_to_bulbs();
                                }
                            }
                        }
                    }
                }
                Task::none()
            }
            Message::GpuSamplingComplete(result) => {
                self.perf_stats.record_sampling_time(result.gpu_time_ms);
                if !self.is_ambient_active {
                    return Task::none();
                }

                self.last_bulb_update = Some(Instant::now());
                for (region_id, color) in &result.colors {
                    if let Some(region) = self.regions.iter_mut().find(|r| r.id == *region_id) {
                        region.sampled_color = *color;
                    }
                }
                self.dispatch_to_bulbs()
            }
            Message::BulbDispatchComplete(elapsed_ms) => {
                self.perf_stats.record_bulb_dispatch(elapsed_ms);
                Task::none()
            }
            Message::RegionStrategyChanged(id, strategy) => {
                if let Some(region) = self.regions.iter_mut().find(|r| r.id == id) {
                    region.strategy = strategy;
                }
                self.active_profile_name = None;
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
                        self.active_profile_name = None;
                    }
                    RegionMessage::Selected(id) => {
                        self.selected_region = id;
                    }
                }
                Task::none()
            }

            // Profiles (from menu bar pick_list)
            Message::LoadProfile(name) => self.load_profile(&name),
        }
    }

    pub fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        use crate::widget::title_bar;
        use iced::widget::{column, rule};

        let title = match self.windows.get(&window_id) {
            Some(WindowKind::Main) => "Cocuyo",
            Some(WindowKind::Settings) => "Settings",
            Some(WindowKind::BulbSetup) => "Bulb Setup",
            Some(WindowKind::ProfileDialog) => "Profiles",
            #[cfg(target_os = "windows")]
            Some(WindowKind::CapturePicker) => "Select Capture Target",
            None => "",
        };

        let screen_content = match self.windows.get(&window_id) {
            Some(WindowKind::Main) => {
                let frame_info = self.current_frame.as_ref().map(|f| (f.width(), f.height()));
                crate::screen::main_window::view(
                    window_id,
                    self.current_frame.as_ref(),
                    &self.recording_state,
                    frame_info,
                    self.is_ambient_active,
                    self.bulb_setup.has_selected_bulbs(),
                    self.bulb_setup.selected_bulbs().len(),
                    &self.regions,
                    self.selected_region,
                    &self.perf_stats,
                    self.config.show_perf_overlay,
                    &self.config.profiles,
                    self.active_profile_name.as_deref(),
                )
            }
            Some(WindowKind::Settings) => self.settings.view().map(Message::Settings),
            Some(WindowKind::BulbSetup) => self.bulb_setup.view().map(Message::BulbSetup),
            Some(WindowKind::ProfileDialog) => {
                if let Some(ref dialog) = self.profile_dialog {
                    dialog.view().map(Message::ProfileDialog)
                } else {
                    iced::widget::space().into()
                }
            }
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

        #[cfg(feature = "tray")]
        subs.push(
            Subscription::run_with((), crate::tray::tray_subscription).map(Message::TrayEvent),
        );

        Subscription::batch(subs)
    }

    #[cfg(target_os = "linux")]
    fn build_recording_subscription(&self) -> Subscription<Message> {
        let backend = self.settings.selected_backend();

        Subscription::run_with(
            (self.session_id, backend, self.recording_fps_limit),
            cocuyo_platform_linux::recording::recording_subscription,
        )
        .map(Message::RecordingEvent)
    }

    #[cfg(target_os = "windows")]
    fn build_recording_subscription(&self) -> Subscription<Message> {
        let target = self
            .capture_target
            .expect("capture_target must be set before recording");
        Subscription::run_with(
            (self.session_id, target, self.recording_fps_limit),
            cocuyo_platform_windows::recording::recording_subscription,
        )
        .map(Message::RecordingEvent)
    }

    #[cfg(target_os = "macos")]
    fn build_recording_subscription(&self) -> Subscription<Message> {
        Subscription::run_with(
            (
                self.session_id,
                self.recording_fps_limit,
                self.recording_resolution_scale,
            ),
            cocuyo_platform_macos::recording::recording_subscription,
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
            settings::Event::RestartApp => {
                self.spawn_new_instance();
                iced::exit()
            }
            settings::Event::ForceCpuSamplingChanged(val) => {
                self.config.force_cpu_sampling = val;
                self.mark_config_dirty();
                if val {
                    self.sampling_worker = None;
                }
                Task::none()
            }
            settings::Event::BulbUpdateIntervalChanged(ms) => {
                self.config.bulb_update_interval_ms = ms;
                self.active_profile_name = None;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::MinBrightnessChanged(pct) => {
                self.config.min_brightness_percent = pct;
                self.active_profile_name = None;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::WhiteColorTempChanged(temp) => {
                self.config.white_color_temp = temp;
                self.active_profile_name = None;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::MinimizeToTrayChanged(val) => {
                self.config.minimize_to_tray = val;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::CaptureFpsLimitChanged(fps) => {
                self.config.capture_fps_limit = fps;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::CaptureResolutionScaleChanged(scale) => {
                self.config.capture_resolution_scale = scale;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::ShowPerfOverlayChanged(val) => {
                self.config.show_perf_overlay = val;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::SmoothTransitionsChanged(val) => {
                self.config.smooth_transitions = val;
                if !val {
                    self.color_smoother.clear();
                }
                self.mark_config_dirty();
                Task::none()
            }
        }
    }

    fn spawn_new_instance(&self) {
        match std::env::current_exe() {
            Ok(exe) => {
                tracing::info!("Spawning new instance: {:?}", exe);
                if let Err(e) = std::process::Command::new(&exe)
                    .args(std::env::args_os().skip(1))
                    .spawn()
                {
                    tracing::error!("Failed to spawn new instance: {}", e);
                }
            }
            Err(e) => tracing::error!("Failed to get current executable path: {}", e),
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
                self.active_profile_name = None;
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
                        cocuyo_platform_windows::dx12_import::reset_d3d_shared_import_failed();
                        self.begin_recording();
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

    fn handle_profile_dialog_event(
        &mut self,
        event: crate::screen::profile_dialog::ProfileDialogEvent,
    ) -> Task<Message> {
        use crate::screen::profile_dialog::ProfileDialogEvent;
        match event {
            ProfileDialogEvent::Save(name) => {
                self.save_profile(&name);
                Task::none()
            }
            ProfileDialogEvent::Load(name) => self.load_profile(&name),
            ProfileDialogEvent::Delete(name) => {
                self.config.profiles.retain(|p| p.name != name);
                if self.active_profile_name.as_deref() == Some(&name) {
                    self.active_profile_name = None;
                }
                self.mark_config_dirty();
                Task::none()
            }
        }
    }

    fn current_or_last_frame_size(&self) -> (f32, f32) {
        self.current_frame
            .as_ref()
            .map(|f| (f.width(), f.height()))
            .or(self.last_frame_size)
            .map(|(w, h)| (w as f32, h as f32))
            .unwrap_or(DEFAULT_FRAME_SIZE)
    }

    fn save_profile(&mut self, name: &str) {
        // Belt-and-suspenders: the dialog disables Save when no frame has been
        // seen, but guard here too so coordinates can't be normalized against
        // DEFAULT_FRAME_SIZE and silently corrupt the profile.
        if self.last_frame_size.is_none() {
            tracing::warn!("Refusing to save profile {name:?}: no frame captured yet");
            return;
        }
        let (frame_w, frame_h) = self.current_or_last_frame_size();

        let profile = crate::config::Profile {
            name: name.to_string(),
            regions: self
                .regions
                .iter()
                .map(|r| crate::config::ProfileRegion::from_region(r, frame_w, frame_h))
                .collect(),
            selected_bulb_macs: self.bulb_setup.selected_bulbs().iter().cloned().collect(),
            bulb_update_interval_ms: self.config.bulb_update_interval_ms,
            min_brightness_percent: self.config.min_brightness_percent,
            white_color_temp: self.config.white_color_temp,
        };

        if let Some(existing) = self.config.profiles.iter_mut().find(|p| p.name == name) {
            *existing = profile;
        } else {
            self.config.profiles.push(profile);
        }

        self.active_profile_name = Some(name.to_string());
        self.mark_config_dirty();
    }

    fn load_profile(&mut self, name: &str) -> Task<Message> {
        let Some(profile) = self
            .config
            .profiles
            .iter()
            .find(|p| p.name == name)
            .cloned()
        else {
            return Task::none();
        };

        self.config.bulb_update_interval_ms = profile.bulb_update_interval_ms;
        self.config.min_brightness_percent = profile.min_brightness_percent;
        self.config.white_color_temp = profile.white_color_temp;

        // Keep all profile MACs even if not currently discovered, so a profile
        // loaded before discovery (or with an offline bulb) is preserved.
        self.bulb_setup
            .set_selected_bulbs(profile.selected_bulb_macs.iter().cloned());
        self.save_bulb_config();

        let (frame_w, frame_h) = self.current_or_last_frame_size();

        self.regions.clear();
        for pr in profile.regions.iter() {
            let region = pr.to_region(self.next_region_id, frame_w, frame_h);
            self.next_region_id += 1;
            self.regions.push(region);
        }
        self.selected_region = None;

        self.active_profile_name = Some(name.to_string());

        self.settings.sync_ambient_from_config(&self.config);
        // Profile switches from the menu pick_list never trigger a window
        // close, so flush now instead of waiting for the usual save points.
        self.mark_config_dirty();
        self.flush_config();
        Task::none()
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

    fn begin_recording(&mut self) {
        self.is_recording = true;
        self.session_id += 1;
        self.recording_fps_limit = self.config.capture_fps_limit;
        self.recording_resolution_scale = self.config.capture_resolution_scale;
    }

    fn graceful_shutdown(&mut self) -> Task<Message> {
        self.flush_config();
        if (self.is_ambient_active || self.is_recording)
            && let Some(cmd_tx) = self.recording_cmd_tx.take()
        {
            let _ = cmd_tx.try_send(RecordingCommand::Stop);
        }
        if let Some(states) = self.saved_bulb_states.take() {
            Task::perform(crate::ambient::restore_bulb_states(states), |()| {
                Message::ExitApp
            })
        } else {
            iced::exit()
        }
    }

    fn dispatch_to_bulbs(&mut self) -> Task<Message> {
        // Preview overlay reads sampled_color as ground truth, so restore it
        // after smoothing for the build call.
        let originals: Option<Vec<Option<(u8, u8, u8)>>> = if self.config.smooth_transitions {
            let snap: Vec<_> = self.regions.iter().map(|r| r.sampled_color).collect();
            for region in &mut self.regions {
                if let Some(rgb) = region.sampled_color {
                    region.sampled_color = Some(self.color_smoother.smooth(&region.bulb_mac, rgb));
                }
            }
            self.color_smoother.mark_updated();
            Some(snap)
        } else {
            None
        };

        let result = crate::ambient::build_bulb_targets(
            &self.regions,
            self.bulb_setup.discovered_bulbs(),
            self.config.min_brightness_percent,
            self.config.white_color_temp,
            &self.last_sent_colors,
        );

        if let Some(orig) = originals {
            for (region, o) in self.regions.iter_mut().zip(orig) {
                region.sampled_color = o;
            }
        }

        if let Some((targets, new_entries)) = result {
            for (mac, color, brightness) in new_entries {
                self.last_sent_colors.insert(mac, (color, brightness));
            }
            let dispatch_start = Instant::now();
            Task::perform(crate::ambient::dispatch_bulb_colors(targets), move |()| {
                Message::BulbDispatchComplete(dispatch_start.elapsed().as_secs_f64() * 1000.0)
            })
        } else {
            Task::none()
        }
    }

    fn sync_regions_to_bulbs(&mut self) {
        let selected_macs: Vec<String> = self.bulb_setup.selected_bulbs().iter().cloned().collect();
        self.regions.retain(|r| selected_macs.contains(&r.bulb_mac));
        self.color_smoother
            .retain(|mac| selected_macs.iter().any(|m| m == mac));

        if let Some(sel) = self.selected_region
            && !self.regions.iter().any(|r| r.id == sel)
        {
            self.selected_region = None;
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
                .unwrap_or(DEFAULT_FRAME_SIZE);

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
                strategy: BoxedStrategy::default(),
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
        if self.find_window_id(kind).is_some() {
            return Task::none();
        }
        let (_id, open) = window::open(window::Settings {
            size,
            min_size: Some(min_size),
            decorations: false,
            transparent: true,
            parent,
            icon: self.window_icon.clone(),
            #[cfg(target_os = "windows")]
            platform_specific: PlatformSpecific {
                corner_preference: CornerPreference::Round,
                ..Default::default()
            },
            ..Default::default()
        });
        open.map(move |id| Message::WindowOpened(id, kind))
    }

    fn save_bulb_config(&mut self) {
        self.config.saved_bulbs = self.bulb_setup.discovered_bulbs().to_vec();
        self.config.selected_bulb_macs = self.bulb_setup.selected_bulbs().iter().cloned().collect();
        self.mark_config_dirty();
    }

    fn mark_config_dirty(&mut self) {
        self.config_dirty = true;
    }

    fn flush_config(&mut self) {
        if self.config_dirty {
            self.config.save();
            self.config_dirty = false;
        }
    }
}
