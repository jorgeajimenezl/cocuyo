use iced::widget::{center, column, text};
use iced::Center;

use crate::app::Message;
use crate::widget::Element;

pub fn view<'a>() -> Element<'a, Message> {
    center(
        column![
            text("Screen Preview").size(24),
            text("(Placeholder -- preview will be rendered here)").size(14),
        ]
        .spacing(10)
        .align_x(Center),
    )
    .into()
}
