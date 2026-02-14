use iced::widget::{center, column, image, rule, text};
use iced::window;
use iced::{Center, ContentFit, Fill};

use crate::app::{FrameData, Message};
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(window_id: window::Id, frame: Option<&FrameData>) -> Element<'a, Message> {
    let content: Element<'a, Message> = match frame {
        Some(f) => image(image::Handle::from_rgba(f.width, f.height, f.data.clone()))
            .content_fit(ContentFit::Contain)
            .width(Fill)
            .height(Fill)
            .into(),
        None => center(
            column![
                text("Waiting for capture...").size(24).color(theme::GREEN),
                text("Start recording to see the preview")
                    .size(14)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(10)
            .align_x(Center),
        )
        .into(),
    };

    column![
        title_bar::view(window_id, "Preview"),
        rule::horizontal(1).style(theme::pixel_rule),
        content,
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
