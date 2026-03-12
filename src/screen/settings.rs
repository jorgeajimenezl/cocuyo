use iced::widget::{rule, scrollable};
use iced::widget::{button, column, container, pick_list, row, slider, text, toggler, tooltip};
use iced::{Fill, Task, padding};

use crate::adapters::{self, GpuAdapter, GpuAdapterSelection};
use crate::config::AppConfig;
#[cfg(target_os = "linux")]
use crate::platform::linux::gst_pipeline::GpuBackend;
use crate::theme;

type Element<'a> = iced::Element<'a, Message, iced::Theme, iced::Renderer>;

#[derive(Debug, Clone)]
pub enum Message {
    #[cfg(target_os = "linux")]
    BackendSelected(usize),
    AdapterSelected(GpuAdapterSelection),
    RestartApp,
    ForceCpuSamplingToggled(bool),
    BulbUpdateIntervalChanged(f32),
    MinBrightnessChanged(f32),
    WhiteColorTempChanged(f32),
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    MinimizeToTrayToggled(bool),
    CaptureFpsLimitChanged(f32),
    CaptureResolutionScaleChanged(f32),
    ShowPerfOverlayToggled(bool),
}

#[derive(Debug, Clone)]
pub enum Event {
    #[cfg(target_os = "linux")]
    BackendChanged(Option<String>),
    AdapterChanged(Option<GpuAdapter>),
    RestartApp,
    ForceCpuSamplingChanged(bool),
    BulbUpdateIntervalChanged(u64),
    MinBrightnessChanged(u8),
    WhiteColorTempChanged(u16),
    MinimizeToTrayChanged(bool),
    CaptureFpsLimitChanged(u32),
    CaptureResolutionScaleChanged(u32),
    ShowPerfOverlayChanged(bool),
}

pub struct Settings {
    #[cfg(target_os = "linux")]
    available_backends: Vec<GpuBackend>,
    #[cfg(target_os = "linux")]
    selected_backend_index: usize,
    available_adapters: Vec<GpuAdapter>,
    selected_adapter: GpuAdapterSelection,
    active_adapter_preference: Option<GpuAdapter>,
    force_cpu_sampling: bool,
    bulb_update_interval_ms: u64,
    min_brightness_percent: u8,
    white_color_temp: u16,
    minimize_to_tray: bool,
    capture_fps_limit: u32,
    capture_resolution_scale: u32,
    show_perf_overlay: bool,
}

impl Settings {
    pub fn new(config: &AppConfig) -> Self {
        #[cfg(target_os = "linux")]
        let (available_backends, selected_backend_index) = {
            use crate::platform::linux::gst_pipeline;
            use tracing::info;

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
            adapters::resolve_selection(config.preferred_adapter.as_ref(), &available_adapters);
        let active_adapter_preference = config.preferred_adapter.clone();

        Self {
            #[cfg(target_os = "linux")]
            available_backends,
            #[cfg(target_os = "linux")]
            selected_backend_index,
            available_adapters,
            selected_adapter,
            active_adapter_preference,
            force_cpu_sampling: config.force_cpu_sampling,
            bulb_update_interval_ms: config.bulb_update_interval_ms,
            min_brightness_percent: config.min_brightness_percent,
            white_color_temp: config.white_color_temp,
            minimize_to_tray: config.minimize_to_tray,
            capture_fps_limit: config.capture_fps_limit,
            capture_resolution_scale: config.capture_resolution_scale,
            show_perf_overlay: config.show_perf_overlay,
        }
    }

    pub fn update(&mut self, message: Message) -> (Task<Message>, Option<Event>) {
        match message {
            #[cfg(target_os = "linux")]
            Message::BackendSelected(idx) => {
                self.selected_backend_index = idx;
                let config_key = self.available_backends.get(idx).map(|b| b.config_key());
                (Task::none(), Some(Event::BackendChanged(config_key)))
            }
            Message::AdapterSelected(selection) => {
                self.selected_adapter = selection.clone();
                let preferred = match &selection {
                    GpuAdapterSelection::Auto => None,
                    GpuAdapterSelection::Named(adapter) => Some(adapter.clone()),
                };
                (Task::none(), Some(Event::AdapterChanged(preferred)))
            }
            Message::RestartApp => (Task::none(), Some(Event::RestartApp)),
            Message::ForceCpuSamplingToggled(val) => {
                self.force_cpu_sampling = val;
                (Task::none(), Some(Event::ForceCpuSamplingChanged(val)))
            }
            Message::BulbUpdateIntervalChanged(val) => {
                self.bulb_update_interval_ms = val as u64;
                (
                    Task::none(),
                    Some(Event::BulbUpdateIntervalChanged(val as u64)),
                )
            }
            Message::MinBrightnessChanged(val) => {
                self.min_brightness_percent = val as u8;
                (Task::none(), Some(Event::MinBrightnessChanged(val as u8)))
            }
            Message::WhiteColorTempChanged(val) => {
                self.white_color_temp = val as u16;
                (Task::none(), Some(Event::WhiteColorTempChanged(val as u16)))
            }
            Message::MinimizeToTrayToggled(val) => {
                self.minimize_to_tray = val;
                (Task::none(), Some(Event::MinimizeToTrayChanged(val)))
            }
            Message::CaptureFpsLimitChanged(val) => {
                self.capture_fps_limit = val as u32;
                (
                    Task::none(),
                    Some(Event::CaptureFpsLimitChanged(val as u32)),
                )
            }
            Message::CaptureResolutionScaleChanged(val) => {
                self.capture_resolution_scale = val as u32;
                (
                    Task::none(),
                    Some(Event::CaptureResolutionScaleChanged(val as u32)),
                )
            }
            Message::ShowPerfOverlayToggled(val) => {
                self.show_perf_overlay = val;
                (Task::none(), Some(Event::ShowPerfOverlayChanged(val)))
            }
        }
    }

    pub fn view(&self) -> Element<'_> {
        let general_section = self.build_general_section();
        let adapter_col = self.build_adapter_section();
        let sampling_section = self.build_sampling_section();
        let ambient_section = self.build_ambient_section();

        #[cfg(target_os = "linux")]
        let content = {
            let backend_section = self.build_backend_section();
            column![
                general_section,
                rule::horizontal(1).style(theme::styled_rule),
                adapter_col,
                rule::horizontal(1).style(theme::styled_rule),
                backend_section,
                rule::horizontal(1).style(theme::styled_rule),
                sampling_section,
                rule::horizontal(1).style(theme::styled_rule),
                ambient_section,
            ]
            .spacing(20)
            .width(Fill)
            .padding(padding::all(20))
        };

        #[cfg(not(target_os = "linux"))]
        let content = column![
            general_section,
            rule::horizontal(1).style(theme::styled_rule),
            adapter_col,
            rule::horizontal(1).style(theme::styled_rule),
            sampling_section,
            rule::horizontal(1).style(theme::styled_rule),
            ambient_section,
        ]
        .spacing(20)
        .width(Fill)
        .padding(padding::all(20));

        container(scrollable(content))
            .width(Fill)
            .height(Fill)
            .style(theme::styled_container)
            .into()
    }

    #[cfg(target_os = "linux")]
    pub fn selected_backend(&self) -> GpuBackend {
        self.available_backends
            .get(self.selected_backend_index)
            .cloned()
            .unwrap_or(GpuBackend::Cpu)
    }

    fn build_general_section(&self) -> iced::widget::Column<'_, Message> {
        let mut col = iced::widget::Column::new().spacing(10);
        col = col.push(text("General").size(18).color(theme::TEXT));

        #[cfg(not(target_os = "linux"))]
        {
            col = col.push(
                toggler(self.minimize_to_tray)
                    .label("Minimize to Tray")
                    .on_toggle(Message::MinimizeToTrayToggled),
            );
            col = col.push(
                text("Keep the app running in the system tray when the main window is closed.")
                    .size(12)
                    .color(theme::TEXT_DIM),
            );
        }

        col = col.push(
            toggler(self.show_perf_overlay)
                .label("Performance Overlay")
                .on_toggle(Message::ShowPerfOverlayToggled),
        );
        col = col.push(
            text("Show capture FPS, sampling time, and bulb dispatch latency on the video preview.")
                .size(12)
                .color(theme::TEXT_DIM),
        );

        col
    }

    fn build_adapter_section(&self) -> iced::widget::Column<'_, Message> {
        let active_label = match &self.active_adapter_preference {
            None => "Currently active: Auto (wgpu default)".to_string(),
            Some(adapter) => format!("Currently active: {}", adapter),
        };

        let pending_restart = match (
            &self.selected_adapter,
            self.active_adapter_preference.as_ref(),
        ) {
            (GpuAdapterSelection::Auto, None) => false,
            (GpuAdapterSelection::Named(adapter), Some(active)) => {
                !adapter.name.eq_ignore_ascii_case(&active.name)
            }
            _ => true,
        };

        let adapter_options = adapters::build_picker_options(&self.available_adapters);
        let mut adapter_col = column![
            row![
                text("GPU Adapter").size(18).color(theme::TEXT),
                tooltip(
                    "🛈",
                    container(
                        #[cfg(target_os = "linux")]
                        text(
                            "On hybrid GPU systems, selecting the correct adapter can improve performance \
                            and compatibility. If unsure, start with 'Auto' or match the adapter used by \
                            your Wayland compositor.",
                        ),
                        #[cfg(not(target_os = "linux"))]
                        text(
                            "Selecting the correct GPU adapter can improve performance and compatibility. \
                            If unsure, 'Auto' is a good choice.",
                        )
                    )
                    .padding(10)
                    .style(container::rounded_box),
                    tooltip::Position::Bottom,
                )
                .style(theme::styled_tooltip)
            ]
            .spacing(5),
            pick_list(
                adapter_options,
                Some(&self.selected_adapter),
                Message::AdapterSelected,
            )
            .style(theme::styled_pick_list)
            .width(Fill),
            text(active_label).size(12).color(theme::TEXT_DIM),
        ]
        .spacing(10);

        if pending_restart {
            adapter_col = adapter_col.push(
                row![
                    text("Restart required for this change to take effect.")
                        .size(12)
                        .color(theme::WARNING),
                    button(text("Restart now").size(12))
                        .on_press(Message::RestartApp)
                        .style(theme::styled_button)
                        .padding([4, 12]),
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            );
        }

        adapter_col
    }

    #[cfg(target_os = "linux")]
    fn build_backend_section(&self) -> iced::widget::Column<'_, Message> {
        let available_backends = &self.available_backends;
        let selected_backend = self.available_backends.get(self.selected_backend_index);
        column![
            text("Video Processing").size(18).color(theme::TEXT),
            pick_list(
                available_backends.as_slice(),
                selected_backend,
                |backend: GpuBackend| {
                    let idx = available_backends
                        .iter()
                        .position(|b| b == &backend)
                        .unwrap_or(0);
                    Message::BackendSelected(idx)
                },
            )
            .style(theme::styled_pick_list)
            .width(Fill),
            text("Select the GPU backend for video format conversion. Changes take effect on the next recording session.")
                .size(12)
                .color(theme::TEXT_DIM),
        ]
        .spacing(10)
    }

    fn build_sampling_section(&self) -> iced::widget::Column<'_, Message> {
        column![
            text("Sampling").size(18).color(theme::TEXT),
            column![
                text(if self.capture_fps_limit == 0 {
                    "Capture Frame Rate: Unlimited".to_string()
                } else {
                    format!("Capture Frame Rate: {} fps", self.capture_fps_limit)
                })
                .size(14)
                .color(theme::TEXT),
                slider(
                    0.0..=60.0,
                    self.capture_fps_limit as f32,
                    Message::CaptureFpsLimitChanged,
                )
                .step(5.0),
                text("Limit how many frames per second are processed. Lower values reduce CPU/GPU usage. 0 = unlimited.")
                    .size(12)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(5),
            column![
                text(if self.capture_resolution_scale >= 100 {
                    "Capture Resolution: Native".to_string()
                } else {
                    format!("Capture Resolution: {}%", self.capture_resolution_scale)
                })
                .size(14)
                .color(theme::TEXT),
                slider(
                    25.0..=100.0,
                    self.capture_resolution_scale as f32,
                    Message::CaptureResolutionScaleChanged,
                )
                .step(25.0),
                text("Downsample captured frames to reduce GPU/CPU usage. Lower values improve performance on older hardware. Changes take effect on next recording session.")
                    .size(12)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(5),
            toggler(self.force_cpu_sampling)
                .label("Force CPU Sampling")
                .on_toggle(Message::ForceCpuSamplingToggled),
            text(
                "Disable GPU compute shaders for color sampling. Use if you experience GPU issues."
            )
            .size(12)
            .color(theme::TEXT_DIM),
        ]
        .spacing(10)
    }

    fn build_ambient_section(&self) -> iced::widget::Column<'_, Message> {
        column![
            text("Ambient Lighting").size(18).color(theme::TEXT),
            column![
                text(format!(
                    "Bulb Update Interval: {}ms",
                    self.bulb_update_interval_ms
                ))
                .size(14)
                .color(theme::TEXT),
                slider(
                    50.0..=500.0,
                    self.bulb_update_interval_ms as f32,
                    Message::BulbUpdateIntervalChanged,
                )
                .step(10.0),
                text("How often colors are sent to bulbs. Lower = more responsive, higher = less network traffic.")
                    .size(12)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(5),
            column![
                text(format!(
                    "Minimum Brightness: {}%",
                    self.min_brightness_percent
                ))
                .size(14)
                .color(theme::TEXT),
                slider(
                    0.0..=100.0,
                    self.min_brightness_percent as f32,
                    Message::MinBrightnessChanged,
                )
                .step(1.0),
                text("Minimum bulb brightness. Set to 0% to allow bulbs to turn fully off.")
                    .size(12)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(5),
            column![
                text(format!("White Color Temperature: {}K", self.white_color_temp))
                    .size(14)
                    .color(theme::TEXT),
                slider(
                    2700.0..=6500.0,
                    self.white_color_temp as f32,
                    Message::WhiteColorTempChanged,
                )
                .step(100.0),
                text("Color temperature used when the sampled color is white or black.")
                    .size(12)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(5),
        ]
        .spacing(15)
    }
}
