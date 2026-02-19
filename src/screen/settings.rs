use iced::widget::{column, container, pick_list, rule, text};
use iced::window;
use iced::{Fill, padding};

use crate::adapters::{AdapterSelection, GpuAdapterInfo};
use crate::app::Message;
use crate::platform::linux::gst_pipeline::GpuBackend;
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(
    window_id: window::Id,
    available_backends: &'a [GpuBackend],
    selected_backend: Option<&'a GpuBackend>,
    available_adapters: &'a [GpuAdapterInfo],
    selected_adapter: &'a AdapterSelection,
    active_adapter_preference: Option<&'a str>,
) -> Element<'a, Message> {
    // Adapter section
    let active_label = match active_adapter_preference {
        None => "Currently active: Auto (wgpu default)".to_string(),
        Some(name) => format!("Currently active: {}", name),
    };

    let pending_restart = match (selected_adapter, active_adapter_preference) {
        (AdapterSelection::Auto, None) => false,
        (AdapterSelection::Named(info), Some(active)) => {
            info.name.to_lowercase() != active.to_lowercase()
        }
        _ => true,
    };

    let adapter_options = crate::adapters::build_picker_options(available_adapters);
    let mut adapter_col = column![
        text("GPU Adapter").size(18).color(theme::TEXT),
        pick_list(
            adapter_options,
            Some(selected_adapter),
            Message::AdapterSelected,
        )
        .style(theme::styled_pick_list),
        text(active_label).size(12).color(theme::TEXT_DIM),
        text("On hybrid GPU systems, select the adapter that matches your Wayland compositor.")
            .size(12)
            .color(theme::TEXT_DIM),
    ]
    .spacing(10);

    if pending_restart {
        adapter_col = adapter_col.push(
            text("Restart required for this change to take effect.")
                .size(12)
                .color(theme::WARNING),
        );
    }

    // Backend section (existing)
    let backend_section = column![
        text("Video Processing").size(18).color(theme::TEXT),
        pick_list(
            available_backends,
            selected_backend,
            |backend: GpuBackend| {
                let idx = available_backends
                    .iter()
                    .position(|b| b == &backend)
                    .unwrap_or(0);
                Message::BackendSelected(idx)
            },
        )
        .style(theme::styled_pick_list),
        text("Select the GPU backend for video format conversion. Changes take effect on the next recording session.")
            .size(12)
            .color(theme::TEXT_DIM),
    ]
    .spacing(10);

    column![
        title_bar::view(window_id, "Settings"),
        rule::horizontal(1).style(theme::styled_rule),
        container(
            column![
                adapter_col,
                rule::horizontal(1).style(theme::styled_rule),
                backend_section,
            ]
            .spacing(20)
            .width(Fill)
            .padding(padding::all(20)),
        )
        .width(Fill)
        .height(Fill)
        .style(theme::styled_container),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
