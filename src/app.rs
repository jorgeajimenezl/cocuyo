use std::collections::BTreeMap;

use iced::widget::container;
use iced::window;

use iced::{Fill, Size, Subscription, Task, Theme};

use crate::config::AppConfig;
use crate::screen::WindowKind;
use crate::screen::{bulb_setup, main_window, settings, profile_dialog};
use crate::widget::Element;

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

#[cfg(target_os = "windows")]
use {
    crate::screen::capture_picker,
    cocuyo_platform_windows::capture_target::CaptureTarget,
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

    // Delegated screens
    MainWindow(main_window::Message),
    Settings(settings::Message),
    BulbSetup(bulb_setup::Message),
    ProfileDialog(profile_dialog::Message),
    #[cfg(target_os = "windows")]
    CapturePicker(capture_picker::Message),

    #[cfg_attr(not(feature = "tray"), allow(dead_code))]
    TrayEvent(crate::tray::TrayAction),

    ExitApp,
    Noop,
}

pub struct Cocuyo {
    windows: BTreeMap<window::Id, WindowKind>,
    config: AppConfig,
    config_dirty: bool,

    // Main window state
    main: main_window::MainWindow,

    // Other screens
    settings: settings::Settings,
    bulb_setup: bulb_setup::BulbSetup,
    profile_dialog: Option<profile_dialog::ProfileDialog>,

    // Windows-specific
    #[cfg(target_os = "windows")]
    capture_picker_dialog: Option<capture_picker::CapturePickerDialog>,
    #[cfg(target_os = "windows")]
    capture_target: Option<CaptureTarget>,

    // App-level
    window_icon: Option<window::Icon>,
    tray: &'static crate::tray::TrayState,
    tray_hide_requested: bool,
}

impl Cocuyo {
    pub fn new(config: AppConfig, tray: &'static crate::tray::TrayState) -> (Self, Task<Message>) {
        let mut main = main_window::MainWindow::new();
        let bulb_setup = bulb_setup::BulbSetup::new(&config);
        main.sync_regions_to_bulbs(&bulb_setup);

        let app = Self {
            windows: BTreeMap::new(),
            config_dirty: false,
            main,
            settings: settings::Settings::new(&config),
            bulb_setup,
            profile_dialog: None,
            window_icon: window::icon::from_rgba(
                include_bytes!(concat!(env!("OUT_DIR"), "/icon-window-256.rgba")).to_vec(),
                256,
                256,
            )
            .ok(),
            tray,
            tray_hide_requested: false,
            #[cfg(target_os = "windows")]
            capture_picker_dialog: None,
            #[cfg(target_os = "windows")]
            capture_target: None,
            config,
        };

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
                    self.capture_picker_dialog = None;
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
                        self.tray
                            .update_menu_text(false, self.main.is_ambient_active());
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
            Message::MainWindow(msg) => {
                let (task, event) = self.main.update(msg, &self.config, &self.bulb_setup);
                let event_task = event.map(|e| self.handle_main_window_event(e));
                Self::merge_tasks(task, Message::MainWindow, event_task)
            }
            Message::Settings(msg) => {
                let (task, event) = self.settings.update(msg);
                let event_task = event.map(|e| self.handle_settings_event(e));
                Self::merge_tasks(task, Message::Settings, event_task)
            }
            Message::BulbSetup(msg) => {
                let (task, event) = self.bulb_setup.update(msg);
                let event_task = event.map(|e| self.handle_bulb_setup_event(e));
                Self::merge_tasks(task, Message::BulbSetup, event_task)
            }
            Message::ProfileDialog(msg) => {
                let Some(dialog) = self.profile_dialog.as_mut() else {
                    return Task::none();
                };
                let (task, event) = dialog.update(msg);
                let event_task = event.map(|e| self.handle_profile_dialog_event(e));
                Self::merge_tasks(task, Message::ProfileDialog, event_task)
            }
            #[cfg(target_os = "windows")]
            Message::CapturePicker(msg) => {
                let Some(picker) = self.capture_picker_dialog.as_mut() else {
                    return Task::none();
                };
                let (task, event) = picker.update(msg);
                let event_task = event.map(|e| self.handle_capture_picker_event(e));
                Self::merge_tasks(task, Message::CapturePicker, event_task)
            }
            Message::TrayEvent(action) => {
                use crate::tray::TrayAction;
                match action {
                    TrayAction::ToggleWindow => {
                        if let Some(id) = self.find_window_id(WindowKind::Main) {
                            self.tray_hide_requested = true;
                            self.tray
                                .update_menu_text(false, self.main.is_ambient_active());
                            window::close(id)
                        } else {
                            self.tray
                                .update_menu_text(true, self.main.is_ambient_active());
                            self.open_window(
                                WindowKind::Main,
                                MAIN_WINDOW_SIZE,
                                MAIN_WINDOW_MIN,
                                None,
                            )
                        }
                    }
                    TrayAction::ToggleAmbient => {
                        if self.main.is_ambient_active() {
                            Task::done(Message::MainWindow(main_window::Message::StopAmbient))
                        } else {
                            Task::done(Message::MainWindow(main_window::Message::StartAmbient))
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
            Some(WindowKind::Main) => self
                .main
                .view(window_id, &self.config, &self.bulb_setup)
                .map(Message::MainWindow),
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
                if let Some(ref picker) = self.capture_picker_dialog {
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

        if self.main.is_recording() {
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
        self.main
            .build_recording_subscription(backend)
            .map(Message::MainWindow)
    }

    #[cfg(target_os = "windows")]
    fn build_recording_subscription(&self) -> Subscription<Message> {
        let target = self
            .capture_target
            .expect("capture_target must be set before recording");
        self.main
            .build_recording_subscription(target)
            .map(Message::MainWindow)
    }

    #[cfg(target_os = "macos")]
    fn build_recording_subscription(&self) -> Subscription<Message> {
        self.main
            .build_recording_subscription()
            .map(Message::MainWindow)
    }

    fn merge_tasks<Msg: Send + 'static>(
        inner: Task<Msg>,
        wrap: impl Fn(Msg) -> Message + Send + 'static,
        event_task: Option<Task<Message>>,
    ) -> Task<Message> {
        let task = inner.map(wrap);
        match event_task {
            Some(ev) => Task::batch([task, ev]),
            None => task,
        }
    }

    // --- Event handlers for delegated screens ---

    fn handle_main_window_event(&mut self, event: main_window::Event) -> Task<Message> {
        match event {
            main_window::Event::OpenSettings => {
                let parent = self.find_window_id(WindowKind::Main);
                self.open_window(
                    WindowKind::Settings,
                    SETTINGS_WINDOW_SIZE,
                    SETTINGS_WINDOW_MIN,
                    parent,
                )
            }
            main_window::Event::OpenBulbSetup => {
                let parent = self.find_window_id(WindowKind::Main);
                self.open_window(
                    WindowKind::BulbSetup,
                    BULB_SETUP_WINDOW_SIZE,
                    BULB_SETUP_WINDOW_MIN,
                    parent,
                )
            }
            main_window::Event::OpenProfileDialog => {
                if self.find_window_id(WindowKind::ProfileDialog).is_none() {
                    let can_save = self.main.last_frame_size.is_some();
                    self.profile_dialog = Some(crate::screen::profile_dialog::ProfileDialog::new(
                        &self.config.profiles,
                        self.main.active_profile_name.as_deref(),
                        can_save,
                    ));
                }
                let parent = self.find_window_id(WindowKind::Main);
                self.open_window(
                    WindowKind::ProfileDialog,
                    PROFILE_DIALOG_SIZE,
                    PROFILE_DIALOG_MIN,
                    parent,
                )
            }
            #[cfg(target_os = "windows")]
            main_window::Event::OpenCapturePicker(intent) => {
                self.capture_picker_dialog = Some(capture_picker::CapturePickerDialog::new(intent));
                let parent = self.find_window_id(WindowKind::Main);
                self.open_window(
                    WindowKind::CapturePicker,
                    PICKER_WINDOW_SIZE,
                    PICKER_WINDOW_MIN,
                    parent,
                )
            }
            main_window::Event::LoadProfile(name) => {
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

                self.main
                    .apply_profile(&name, &self.config, &mut self.bulb_setup);
                self.save_bulb_config();

                self.settings.sync_ambient_from_config(&self.config);
                self.mark_config_dirty();
                self.flush_config();
                Task::none()
            }
            main_window::Event::ConfigDirty => {
                self.mark_config_dirty();
                Task::none()
            }
            main_window::Event::TrayMenuDirty => {
                self.tray.update_menu_text(
                    self.find_window_id(WindowKind::Main).is_some(),
                    self.main.is_ambient_active(),
                );
                Task::none()
            }
            main_window::Event::RestoreBulbStates(states) => {
                self.tray
                    .update_menu_text(self.find_window_id(WindowKind::Main).is_some(), false);
                Task::perform(crate::ambient::restore_bulb_states(states), |()| {
                    Message::Noop
                })
            }
        }
    }

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
                self.main.notify_settings_changed(&self.config);
                Task::none()
            }
            settings::Event::BulbUpdateIntervalChanged(ms) => {
                self.config.bulb_update_interval_ms = ms;
                self.main.active_profile_name = None;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::MinBrightnessChanged(pct) => {
                self.config.min_brightness_percent = pct;
                self.main.active_profile_name = None;
                self.mark_config_dirty();
                Task::none()
            }
            settings::Event::WhiteColorTempChanged(temp) => {
                self.config.white_color_temp = temp;
                self.main.active_profile_name = None;
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
                self.main.notify_settings_changed(&self.config);
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
                self.main.sync_regions_to_bulbs(&self.bulb_setup);
                self.save_bulb_config();
                self.close_window_by_kind(WindowKind::BulbSetup)
            }
            bulb_setup::BulbSetupEvent::SelectionChanged => {
                self.main.sync_regions_to_bulbs(&self.bulb_setup);
                self.main.active_profile_name = None;
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
                self.capture_picker_dialog = None;

                let main_task = self
                    .main
                    .handle_capture_target_selected(intent, &self.config, &self.bulb_setup)
                    .map(Message::MainWindow);

                Task::batch([close_task, main_task])
            }
            capture_picker::Event::Cancelled => {
                self.capture_picker_dialog = None;
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
                self.main
                    .save_profile(&name, &mut self.config, &self.bulb_setup);
                self.mark_config_dirty();
                Task::none()
            }
            ProfileDialogEvent::Load(name) => {
                Task::done(Message::MainWindow(main_window::Message::LoadProfile(name)))
            }
            ProfileDialogEvent::Delete(name) => {
                self.config.profiles.retain(|p| p.name != name);
                if self.main.active_profile_name.as_deref() == Some(&name) {
                    self.main.active_profile_name = None;
                }
                self.mark_config_dirty();
                Task::none()
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

    fn graceful_shutdown(&mut self) -> Task<Message> {
        self.flush_config();
        let saved_states = self.main.handle_shutdown();
        if let Some(states) = saved_states {
            Task::perform(crate::ambient::restore_bulb_states(states), |()| {
                Message::ExitApp
            })
        } else {
            iced::exit()
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
