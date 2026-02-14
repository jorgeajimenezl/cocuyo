use iced::widget::{column, container, pick_list, text};
use iced::{Fill, padding};

use crate::app::Message;
use crate::gst_pipeline::GpuBackend;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(
    available_backends: &'a [GpuBackend],
    selected_backend: Option<&'a GpuBackend>,
) -> Element<'a, Message> {
    let heading = text("Settings").size(24).color(theme::GREEN);

    let backend_section = column![
        text("Video Processing").size(18).color(theme::GREEN),
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
        .style(theme::pixel_pick_list),
        text("Select the GPU backend for video format conversion. Changes take effect on the next recording session.")
            .size(12)
            .color(theme::TEXT_DIM),
    ]
    .spacing(10);

    container(
        column![heading, backend_section]
            .spacing(20)
            .width(Fill)
            .padding(padding::all(20)),
    )
    .style(theme::pixel_container)
    .into()
}
