use iced::widget::{center, column, rule, text};
use iced::window;
use iced::{Center, Fill};

use crate::app::Message;
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(window_id: window::Id) -> Element<'a, Message> {
    column![
        title_bar::view(window_id, "Preview"),
        rule::horizontal(1).style(theme::pixel_rule),
        center(
            column![
                text("Screen Preview").size(24).color(theme::GREEN),
                text("(Placeholder -- preview will be rendered here)")
                    .size(14)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(10)
            .align_x(Center),
        ),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
