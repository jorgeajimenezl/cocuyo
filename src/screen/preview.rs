use iced::widget::{center, column, text};
use iced::Center;

use crate::app::Message;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>() -> Element<'a, Message> {
    center(
        column![
            text("Screen Preview").size(24).color(theme::GREEN),
            text("(Placeholder -- preview will be rendered here)")
                .size(14)
                .color(theme::TEXT_DIM),
        ]
        .spacing(10)
        .align_x(Center),
    )
    .into()
}
