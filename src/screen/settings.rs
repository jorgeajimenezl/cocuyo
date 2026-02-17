use iced::widget::{column, container, pick_list, rule, text};
use iced::window;
use iced::{Fill, padding};

use crate::app::Message;
use crate::platform::linux::gst_pipeline::GpuBackend;
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(
    window_id: window::Id,
    available_backends: &'a [GpuBackend],
    selected_backend: Option<&'a GpuBackend>,
) -> Element<'a, Message> {
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
            column![backend_section]
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
