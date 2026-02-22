use iced::widget::{column, container, pick_list, row, text, tooltip};
#[cfg(target_os = "linux")]
use iced::widget::rule;
use iced::{Fill, Task, padding};

use crate::adapters::{self, GpuAdapterSelection};
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
}

#[derive(Debug, Clone)]
pub enum Event {
    #[cfg(target_os = "linux")]
    BackendChanged(Option<String>),
    AdapterChanged(Option<String>),
}

pub struct Settings {
    #[cfg(target_os = "linux")]
    available_backends: Vec<GpuBackend>,
    #[cfg(target_os = "linux")]
    selected_backend_index: usize,
    available_adapters: Vec<String>,
    selected_adapter: GpuAdapterSelection,
    active_adapter_preference: Option<String>,
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
            adapters::resolve_selection(config.preferred_adapter.as_deref(), &available_adapters);
        let active_adapter_preference = config.preferred_adapter.clone();

        Self {
            #[cfg(target_os = "linux")]
            available_backends,
            #[cfg(target_os = "linux")]
            selected_backend_index,
            available_adapters,
            selected_adapter,
            active_adapter_preference,
        }
    }

    pub fn update(&mut self, message: Message) -> (Task<Message>, Option<Event>) {
        match message {
            #[cfg(target_os = "linux")]
            Message::BackendSelected(idx) => {
                self.selected_backend_index = idx;
                let config_key = self
                    .available_backends
                    .get(idx)
                    .map(|b| b.config_key());
                (Task::none(), Some(Event::BackendChanged(config_key)))
            }
            Message::AdapterSelected(selection) => {
                self.selected_adapter = selection.clone();
                let preferred = match &selection {
                    GpuAdapterSelection::Auto => None,
                    GpuAdapterSelection::Named(name) => Some(name.clone()),
                };
                (Task::none(), Some(Event::AdapterChanged(preferred)))
            }
        }
    }

    pub fn view(&self) -> Element<'_> {
        let adapter_col = self.build_adapter_section();

        #[cfg(target_os = "linux")]
        let content = {
            let backend_section = self.build_backend_section();
            column![
                adapter_col,
                rule::horizontal(1).style(theme::styled_rule),
                backend_section,
            ]
            .spacing(20)
            .width(Fill)
            .padding(padding::all(20))
        };

        #[cfg(not(target_os = "linux"))]
        let content = column![adapter_col]
            .spacing(20)
            .width(Fill)
            .padding(padding::all(20));

        container(content)
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

    fn build_adapter_section(&self) -> iced::widget::Column<'_, Message> {
        let active_label = match &self.active_adapter_preference {
            None => "Currently active: Auto (wgpu default)".to_string(),
            Some(name) => format!("Currently active: {}", name),
        };

        let pending_restart =
            match (&self.selected_adapter, self.active_adapter_preference.as_deref()) {
                (GpuAdapterSelection::Auto, None) => false,
                (GpuAdapterSelection::Named(name), Some(active)) => {
                    name.to_lowercase() != active.to_lowercase()
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
                text("Restart required for this change to take effect.")
                    .size(12)
                    .color(theme::WARNING),
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
}
