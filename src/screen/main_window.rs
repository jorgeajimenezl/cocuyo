use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::{
    button, center, column, container, pick_list, row, rule, scrollable, shader, stack, text,
};
use iced::{Center, Color, Fill, Length, Subscription, Task};
use tokio::sync::mpsc;

use crate::ambient::{BulbColor, ColorSmoother, SavedBulbState};
use crate::config::AppConfig;
use crate::perf_stats::PerfStats;
use crate::screen::bulb_setup::BulbSetup;
use crate::theme;
use crate::widget::Element;
use crate::widget::perf_hud::PerfHud;
use crate::widget::region_overlay::{RegionMessage, RegionOverlay};
use crate::widget::video_shader::VideoScene;
use cocuyo_core::frame::FrameData;
use cocuyo_core::recording::{RecordingCommand, RecordingEvent, RecordingState};
use cocuyo_sampling::gpu::SamplingWorker;
use cocuyo_sampling::{BoxedStrategy, Region};

#[cfg(target_os = "windows")]
use cocuyo_platform_windows::capture_target::{CaptureTarget, PickerIntent};

const DEFAULT_FRAME_SIZE: (f32, f32) = (1920.0, 1080.0);

#[derive(Debug, Clone)]
pub enum Message {
    StartRecording,
    StopRecording,
    StartAmbient,
    StopAmbient,
    BulbStatesSaved(Vec<SavedBulbState>),
    RecordingEvent(RecordingEvent),
    GpuSamplingComplete(cocuyo_sampling::gpu::SamplingResult),
    BulbDispatchComplete(f64),
    RegionUpdate(RegionMessage),
    RegionStrategyChanged(usize, BoxedStrategy),
    LoadProfile(String),
    // UI routing — handled by app via Event
    OpenSettingsPressed,
    OpenBulbSetupPressed,
    OpenProfileDialogPressed,
}

#[derive(Debug, Clone)]
pub enum Event {
    OpenSettings,
    OpenBulbSetup,
    OpenProfileDialog,
    #[cfg(target_os = "windows")]
    OpenCapturePicker(PickerIntent),
    /// App should apply the named profile: mutate config, call apply_profile, sync settings.
    LoadProfile(String),
    ConfigDirty,
    TrayMenuDirty,
    RestoreBulbStates(Vec<SavedBulbState>),
}

pub struct MainWindow {
    // Frame / capture
    pub current_frame: Option<Arc<FrameData>>,
    pub last_frame_size: Option<(u32, u32)>,
    // Recording
    pub recording_state: RecordingState,
    is_recording: bool,
    session_id: u64,
    recording_fps_limit: u32,
    recording_resolution_scale: u32,
    recording_cmd_tx: Option<mpsc::Sender<RecordingCommand>>,
    // Ambient
    is_ambient_active: bool,
    last_bulb_update: Option<Instant>,
    last_sent_colors: HashMap<String, (BulbColor, u8)>,
    saved_bulb_states: Option<Vec<SavedBulbState>>,
    // Regions
    pub regions: Vec<Region>,
    pub next_region_id: usize,
    pub selected_region: Option<usize>,
    // Sampling
    sampling_worker: Option<SamplingWorker>,
    color_smoother: ColorSmoother,
    // Performance stats
    pub perf_stats: PerfStats,
    // Profile
    pub active_profile_name: Option<String>,
}

impl MainWindow {
    pub fn new() -> Self {
        Self {
            current_frame: None,
            last_frame_size: None,
            recording_state: RecordingState::Idle,
            is_recording: false,
            session_id: 0,
            recording_fps_limit: 0,
            recording_resolution_scale: 100,
            recording_cmd_tx: None,
            is_ambient_active: false,
            last_bulb_update: None,
            last_sent_colors: HashMap::new(),
            saved_bulb_states: None,
            regions: Vec::new(),
            next_region_id: 1,
            selected_region: None,
            sampling_worker: None,
            color_smoother: ColorSmoother::new(),
            perf_stats: PerfStats::new(),
            active_profile_name: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    pub fn is_ambient_active(&self) -> bool {
        self.is_ambient_active
    }

    #[cfg(target_os = "linux")]
    pub fn build_recording_subscription(
        &self,
        backend: cocuyo_platform_linux::gst_pipeline::GpuBackend,
    ) -> Subscription<Message> {
        Subscription::run_with(
            (self.session_id, backend, self.recording_fps_limit),
            cocuyo_platform_linux::recording::recording_subscription,
        )
        .map(Message::RecordingEvent)
    }

    #[cfg(target_os = "windows")]
    pub fn build_recording_subscription(&self, target: CaptureTarget) -> Subscription<Message> {
        Subscription::run_with(
            (self.session_id, target, self.recording_fps_limit),
            cocuyo_platform_windows::recording::recording_subscription,
        )
        .map(Message::RecordingEvent)
    }

    #[cfg(target_os = "macos")]
    pub fn build_recording_subscription(&self) -> Subscription<Message> {
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

    pub fn update(
        &mut self,
        msg: Message,
        config: &AppConfig,
        bulb_setup: &BulbSetup,
    ) -> (Task<Message>, Option<Event>) {
        match msg {
            Message::StartRecording => {
                #[cfg(target_os = "linux")]
                {
                    cocuyo_platform_linux::vulkan_dmabuf::reset_dmabuf_import_failed();
                    self.begin_recording(config);
                    return (Task::none(), None);
                }
                #[cfg(target_os = "macos")]
                {
                    cocuyo_platform_macos::metal_import::reset_iosurface_import_failed();
                    self.begin_recording(config);
                    return (Task::none(), None);
                }
                #[cfg(target_os = "windows")]
                {
                    return (
                        Task::none(),
                        Some(Event::OpenCapturePicker(PickerIntent::StartRecording)),
                    );
                }
                #[allow(unreachable_code)]
                (Task::none(), None)
            }
            Message::StopRecording => {
                self.perf_stats.reset();
                if let Some(cmd_tx) = self.recording_cmd_tx.take() {
                    let _ = cmd_tx.try_send(RecordingCommand::Stop);
                }
                self.current_frame = None;
                (Task::none(), None)
            }
            Message::StartAmbient => {
                if !bulb_setup.has_selected_bulbs() {
                    return (Task::none(), None);
                }
                #[cfg(target_os = "windows")]
                {
                    return (
                        Task::none(),
                        Some(Event::OpenCapturePicker(PickerIntent::StartAmbient)),
                    );
                }
                #[allow(unreachable_code)]
                {
                    let bulbs = bulb_setup.selected_bulb_infos();
                    (
                        Task::perform(
                            crate::ambient::save_bulb_states(bulbs),
                            Message::BulbStatesSaved,
                        ),
                        None,
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

                // Lazily spawn GPU sampling worker when ambient starts
                if self.sampling_worker.is_none()
                    && !config.force_cpu_sampling
                    && let Some((device, queue)) = crate::gpu_context::get_gpu_context()
                {
                    self.sampling_worker =
                        Some(SamplingWorker::spawn(device.clone(), queue.clone()));
                }
                if !self.is_recording {
                    #[cfg(target_os = "linux")]
                    cocuyo_platform_linux::vulkan_dmabuf::reset_dmabuf_import_failed();
                    #[cfg(target_os = "windows")]
                    cocuyo_platform_windows::dx12_import::reset_d3d_shared_import_failed();
                    self.begin_recording(config);
                }
                (Task::none(), Some(Event::TrayMenuDirty))
            }
            Message::StopAmbient => {
                self.perf_stats.reset();
                self.is_ambient_active = false;
                self.last_bulb_update = None;
                self.last_sent_colors.clear();
                self.color_smoother.clear();
                self.sampling_worker = None;
                if let Some(cmd_tx) = self.recording_cmd_tx.take() {
                    let _ = cmd_tx.try_send(RecordingCommand::Stop);
                }
                self.current_frame = None;
                let event = if let Some(states) = self.saved_bulb_states.take() {
                    Some(Event::RestoreBulbStates(states))
                } else {
                    Some(Event::TrayMenuDirty)
                };
                (Task::none(), event)
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
                            self.recording_state = state;
                            let event = if let Some(states) = self.saved_bulb_states.take() {
                                Some(Event::RestoreBulbStates(states))
                            } else {
                                Some(Event::TrayMenuDirty)
                            };
                            return (Task::none(), event);
                        }
                        self.recording_state = state;
                    }
                    RecordingEvent::Frame(frame) => {
                        if !self.is_recording {
                            return (Task::none(), None);
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
                                        >= Duration::from_millis(config.bulb_update_interval_ms)
                                })
                                .unwrap_or(true);

                            if should_update {
                                if self.regions.is_empty() {
                                    return (Task::none(), None);
                                }

                                // Try GPU sampling (async, off main thread)
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
                                            SendResult::Sent(task) => return (task, None),
                                            SendResult::Busy => {}
                                            SendResult::Dead => {
                                                tracing::warn!(
                                                    "GPU sampling worker died, falling back to CPU"
                                                );
                                                self.sampling_worker = None;
                                            }
                                        }
                                    }
                                }

                                // CPU fallback
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
                                    return (self.dispatch_to_bulbs(config, bulb_setup), None);
                                }
                            }
                        }
                    }
                }
                (Task::none(), None)
            }
            Message::GpuSamplingComplete(result) => {
                self.perf_stats.record_sampling_time(result.gpu_time_ms);
                if !self.is_ambient_active {
                    return (Task::none(), None);
                }
                self.last_bulb_update = Some(Instant::now());
                for (region_id, color) in &result.colors {
                    if let Some(region) = self.regions.iter_mut().find(|r| r.id == *region_id) {
                        region.sampled_color = *color;
                    }
                }
                (self.dispatch_to_bulbs(config, bulb_setup), None)
            }
            Message::BulbDispatchComplete(elapsed_ms) => {
                self.perf_stats.record_bulb_dispatch(elapsed_ms);
                (Task::none(), None)
            }
            Message::RegionStrategyChanged(id, strategy) => {
                if let Some(region) = self.regions.iter_mut().find(|r| r.id == id) {
                    region.strategy = strategy;
                }
                self.active_profile_name = None;
                (Task::none(), Some(Event::ConfigDirty))
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
                (Task::none(), None)
            }
            Message::LoadProfile(name) => (Task::none(), Some(Event::LoadProfile(name))),
            Message::OpenSettingsPressed => (Task::none(), Some(Event::OpenSettings)),
            Message::OpenBulbSetupPressed => (Task::none(), Some(Event::OpenBulbSetup)),
            Message::OpenProfileDialogPressed => (Task::none(), Some(Event::OpenProfileDialog)),
        }
    }

    pub fn view<'a>(
        &'a self,
        config: &'a AppConfig,
        bulb_setup: &'a BulbSetup,
    ) -> Element<'a, Message> {
        let has_selected_bulbs = bulb_setup.has_selected_bulbs();
        let selected_count = bulb_setup.selected_bulbs().len();
        let frame_info = self.current_frame.as_ref().map(|f| (f.width(), f.height()));

        // Profile dropdown for the menu bar
        let profile_names: Vec<String> = config.profiles.iter().map(|p| p.name.clone()).collect();
        let profile_picker: Element<'_, Message> = if profile_names.is_empty() {
            text("No profiles").size(12).color(theme::TEXT_DIM).into()
        } else {
            pick_list(
                self.active_profile_name.clone(),
                profile_names,
                |s: &String| s.clone(),
            )
            .on_select(Message::LoadProfile)
            .text_size(12)
            .style(theme::styled_pick_list)
            .into()
        };

        let menu_bar = container(
            row![
                button("Bulbs")
                    .on_press(Message::OpenBulbSetupPressed)
                    .style(theme::styled_button),
                button("Settings")
                    .on_press(Message::OpenSettingsPressed)
                    .style(theme::styled_button),
                button("Profiles")
                    .on_press(Message::OpenProfileDialogPressed)
                    .style(theme::styled_button),
                iced::widget::space().width(Fill),
                profile_picker,
            ]
            .spacing(5)
            .padding(5)
            .align_y(Center),
        )
        .width(Fill)
        .style(theme::menu_bar_container);

        // Left panel: video preview + region overlay + perf HUD
        let preview_area: Element<'a, Message> = match (self.current_frame.as_ref(), frame_info) {
            (Some(f), Some((fw, fh))) => {
                let video: Element<'a, Message> = shader(VideoScene::new(Some(Arc::clone(f))))
                    .width(Fill)
                    .height(Fill)
                    .into();

                let mut layers: Vec<Element<'a, Message>> = vec![video];

                if self.is_ambient_active {
                    let overlay =
                        RegionOverlay::new(&self.regions, fw, fh, self.selected_region).view();
                    layers.push(overlay.into());
                }

                if config.show_perf_overlay && self.perf_stats.has_frame_data() {
                    let hud: Element<'a, Message> = PerfHud::new(&self.perf_stats).view().into();
                    layers.push(hud);
                }

                stack(layers).width(Fill).height(Fill).into()
            }
            _ => center(
                column![
                    text("No capture active").size(20).color(theme::TEXT),
                    text("Start preview or ambient mode to see the capture")
                        .size(14)
                        .color(theme::TEXT_DIM),
                ]
                .spacing(10)
                .align_x(Center),
            )
            .width(Fill)
            .height(Fill)
            .into(),
        };

        // Right panel: controls
        let ambient_controls: Element<'_, Message> = if self.is_ambient_active {
            button("Stop Ambient")
                .on_press(Message::StopAmbient)
                .style(theme::styled_button)
                .into()
        } else if has_selected_bulbs {
            button("Start Ambient")
                .on_press(Message::StartAmbient)
                .style(theme::styled_button)
                .into()
        } else {
            text("Select bulbs to enable ambient")
                .size(12)
                .color(theme::TEXT_DIM)
                .into()
        };

        let recording_controls: Element<'_, Message> = if self.is_ambient_active {
            text("Preview controlled by ambient")
                .size(12)
                .color(theme::WARNING)
                .into()
        } else {
            match &self.recording_state {
                RecordingState::Idle => button("Start Preview")
                    .on_press(Message::StartRecording)
                    .style(theme::styled_button)
                    .into(),
                RecordingState::Starting => {
                    text("Starting...").size(12).color(theme::WARNING).into()
                }
                RecordingState::Recording => column![
                    text("Previewing").size(12).color(theme::SUCCESS),
                    button("Stop Preview")
                        .on_press(Message::StopRecording)
                        .style(theme::styled_button),
                ]
                .spacing(8)
                .align_x(Center)
                .into(),
                RecordingState::Error(msg) => column![
                    text(format!("Error: {}", msg))
                        .size(12)
                        .color(theme::DANGER),
                    button("Retry")
                        .on_press(Message::StartRecording)
                        .style(theme::styled_button),
                ]
                .spacing(8)
                .align_x(Center)
                .into(),
            }
        };

        // Region list
        let region_list: Element<'a, Message> = if self.regions.is_empty() {
            text("Select bulbs to create regions")
                .size(12)
                .color(theme::TEXT_DIM)
                .into()
        } else {
            let items: Vec<Element<'a, Message>> = self
                .regions
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let label = format!(
                        "R{} ({})",
                        i + 1,
                        &r.bulb_mac[r.bulb_mac.len().saturating_sub(8)..]
                    );

                    let color_indicator: Element<'a, Message> =
                        if let Some((cr, cg, cb)) = r.sampled_color {
                            container(text(""))
                                .width(14)
                                .height(14)
                                .style(move |_: &_| container::Style {
                                    background: Some(iced::Background::Color(Color::from_rgb8(
                                        cr, cg, cb,
                                    ))),
                                    border: iced::Border {
                                        radius: 2.0.into(),
                                        width: 1.0,
                                        color: theme::BORDER,
                                    },
                                    ..Default::default()
                                })
                                .into()
                        } else {
                            container(text(""))
                                .width(14)
                                .height(14)
                                .style(|_: &_| container::Style {
                                    background: Some(iced::Background::Color(theme::BG_SECONDARY)),
                                    border: iced::Border {
                                        radius: 2.0.into(),
                                        width: 1.0,
                                        color: theme::BORDER,
                                    },
                                    ..Default::default()
                                })
                                .into()
                        };

                    let region_id = r.id;
                    let strategy_picker = pick_list(
                        Some(r.strategy.clone()),
                        cocuyo_sampling::all_strategies(),
                        |s: &cocuyo_sampling::BoxedStrategy| s.to_string(),
                    )
                    .on_select(move |s| Message::RegionStrategyChanged(region_id, s))
                    .text_size(11)
                    .style(theme::styled_pick_list);

                    column![
                        row![color_indicator, text(label).size(12).color(theme::TEXT),]
                            .spacing(5)
                            .align_y(Center),
                        strategy_picker,
                    ]
                    .spacing(3)
                    .into()
                })
                .collect();

            scrollable(column(items).spacing(4).width(Fill))
                .height(Fill)
                .into()
        };

        let mut controls_panel = column![
            text("Controls").size(16).color(theme::TEXT),
            rule::horizontal(1).style(theme::styled_rule),
            ambient_controls,
            rule::horizontal(1).style(theme::styled_rule),
            recording_controls,
            rule::horizontal(1).style(theme::styled_rule),
        ];

        if self.is_ambient_active {
            controls_panel = controls_panel
                .push(text("Regions").size(14).color(theme::TEXT))
                .push(region_list);
        }

        let controls_panel = controls_panel
            .spacing(8)
            .padding(10)
            .width(Length::Fixed(250.0))
            .height(Fill);

        // Status bar
        let status_text = if self.is_ambient_active {
            text(format!(
                "Ambient active -- {} bulb{} -- {} region{}",
                selected_count,
                if selected_count == 1 { "" } else { "s" },
                self.regions.len(),
                if self.regions.len() == 1 { "" } else { "s" },
            ))
            .color(theme::SUCCESS)
        } else {
            match &self.recording_state {
                RecordingState::Idle => {
                    if !has_selected_bulbs {
                        text("No bulbs selected").color(theme::TEXT_DIM)
                    } else {
                        text(format!(
                            "{} bulb{} selected",
                            selected_count,
                            if selected_count == 1 { "" } else { "s" }
                        ))
                        .color(theme::TEXT)
                    }
                }
                RecordingState::Starting => text("Starting preview...").color(theme::WARNING),
                RecordingState::Recording => text("Previewing").color(theme::SUCCESS),
                RecordingState::Error(_) => text("Error").color(theme::DANGER),
            }
        };

        let mut status_row = row![text("Status: ").color(theme::TEXT_DIM), status_text].spacing(5);
        if let Some((w, h)) = frame_info {
            status_row = status_row
                .push(text(" | ").color(theme::TEXT_DIM))
                .push(text(format!("{}x{}", w, h)).color(theme::TEXT));
        }

        let status_bar = container(status_row)
            .padding(5)
            .width(Fill)
            .style(theme::status_bar_container);

        column![
            menu_bar,
            rule::horizontal(1).style(theme::styled_rule),
            row![
                preview_area,
                rule::vertical(1).style(theme::styled_rule),
                controls_panel,
            ]
            .height(Fill),
            rule::horizontal(1).style(theme::styled_rule),
            status_bar,
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }

    /// Called by app after BulbSetup changes to keep regions in sync.
    pub fn sync_regions_to_bulbs(&mut self, bulb_setup: &BulbSetup) {
        let selected_macs: Vec<String> = bulb_setup.selected_bulbs().iter().cloned().collect();
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

    /// Called by app after settings/config changes that affect main window behavior.
    pub fn notify_settings_changed(&mut self, config: &AppConfig) {
        if config.force_cpu_sampling {
            self.sampling_worker = None;
        }
        if !config.smooth_transitions {
            self.color_smoother.clear();
        }
    }

    /// Called by app when the Windows capture picker selects a target.
    #[cfg(target_os = "windows")]
    pub fn handle_capture_target_selected(
        &mut self,
        intent: PickerIntent,
        config: &AppConfig,
        bulb_setup: &BulbSetup,
    ) -> Task<Message> {
        match intent {
            PickerIntent::StartRecording => {
                cocuyo_platform_windows::dx12_import::reset_d3d_shared_import_failed();
                self.begin_recording(config);
                Task::none()
            }
            PickerIntent::StartAmbient => {
                let bulbs = bulb_setup.selected_bulb_infos();
                Task::perform(
                    crate::ambient::save_bulb_states(bulbs),
                    Message::BulbStatesSaved,
                )
            }
        }
    }

    /// Called by app during graceful shutdown. Stops recording and returns any
    /// saved bulb states that need to be restored.
    pub fn handle_shutdown(&mut self) -> Option<Vec<SavedBulbState>> {
        if let Some(cmd_tx) = self.recording_cmd_tx.take() {
            let _ = cmd_tx.try_send(RecordingCommand::Stop);
        }
        self.saved_bulb_states.take()
    }

    /// Save current state as a named profile. Returns false if no frame has
    /// been captured yet (coordinates can't be normalized).
    pub fn save_profile(
        &mut self,
        name: &str,
        config: &mut AppConfig,
        bulb_setup: &BulbSetup,
    ) -> bool {
        if self.last_frame_size.is_none() {
            tracing::warn!("Refusing to save profile {name:?}: no frame captured yet");
            return false;
        }
        let (frame_w, frame_h) = self.current_or_last_frame_size();

        let profile = crate::config::Profile {
            name: name.to_string(),
            regions: self
                .regions
                .iter()
                .map(|r| crate::config::ProfileRegion::from_region(r, frame_w, frame_h))
                .collect(),
            selected_bulb_macs: bulb_setup.selected_bulbs().iter().cloned().collect(),
            bulb_update_interval_ms: config.bulb_update_interval_ms,
            min_brightness_percent: config.min_brightness_percent,
            white_color_temp: config.white_color_temp,
        };

        if let Some(existing) = config.profiles.iter_mut().find(|p| p.name == name) {
            *existing = profile;
        } else {
            config.profiles.push(profile);
        }

        self.active_profile_name = Some(name.to_string());
        true
    }

    /// Apply a loaded profile's region/bulb state. Config fields are mutated
    /// by the caller (app) before this is called.
    pub fn apply_profile(
        &mut self,
        name: &str,
        config: &AppConfig,
        bulb_setup: &mut BulbSetup,
    ) {
        let Some(profile) = config.profiles.iter().find(|p| p.name == name).cloned() else {
            return;
        };

        let (frame_w, frame_h) = self.current_or_last_frame_size();

        bulb_setup.set_selected_bulbs(profile.selected_bulb_macs.iter().cloned());

        self.regions.clear();
        for pr in profile.regions.iter() {
            let region = pr.to_region(self.next_region_id, frame_w, frame_h);
            self.next_region_id += 1;
            self.regions.push(region);
        }
        self.selected_region = None;
        self.active_profile_name = Some(name.to_string());
    }

    // --- Private helpers ---

    fn begin_recording(&mut self, config: &AppConfig) {
        self.is_recording = true;
        self.session_id += 1;
        self.recording_fps_limit = config.capture_fps_limit;
        self.recording_resolution_scale = config.capture_resolution_scale;
        self.recording_state = RecordingState::Starting;
    }

    fn current_or_last_frame_size(&self) -> (f32, f32) {
        self.current_frame
            .as_ref()
            .map(|f| (f.width(), f.height()))
            .or(self.last_frame_size)
            .map(|(w, h)| (w as f32, h as f32))
            .unwrap_or(DEFAULT_FRAME_SIZE)
    }

    fn dispatch_to_bulbs(
        &mut self,
        config: &AppConfig,
        bulb_setup: &BulbSetup,
    ) -> Task<Message> {
        // Preview overlay reads sampled_color as ground truth, so restore it
        // after smoothing for the build call.
        let originals: Option<Vec<Option<(u8, u8, u8)>>> = if config.smooth_transitions {
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
            bulb_setup.discovered_bulbs(),
            config.min_brightness_percent,
            config.white_color_temp,
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
}
