use iced::widget::{center, column, rule, shader, text};
use iced::window;
use iced::{Center, Fill};

use crate::app::{FrameData, Message};
use crate::screen::title_bar;
use crate::screen::video_shader::VideoScene;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(window_id: window::Id, frame: Option<&FrameData>) -> Element<'a, Message> {
    let content: Element<'a, Message> = match frame {
        Some(f) => shader(VideoScene::new(Some(f)))
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
