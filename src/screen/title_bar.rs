use iced::widget::{button, container, mouse_area, row, text};
use iced::window;
use iced::Fill;

use crate::app::Message;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(window_id: window::Id, title: &str) -> Element<'a, Message> {
    let title_text = text(title.to_string())
        .size(14)
        .color(theme::GREEN);

    let minimize_btn = button(text("_").size(12))
        .on_press(Message::MinimizeWindow(window_id))
        .style(theme::pixel_button)
        .padding([2, 8]);

    let maximize_btn = button(text("\u{25a1}").size(12))
        .on_press(Message::MaximizeWindow(window_id))
        .style(theme::pixel_button)
        .padding([2, 8]);

    let close_btn = button(text("\u{2715}").size(12))
        .on_press(Message::CloseWindow(window_id))
        .style(theme::close_button)
        .padding([2, 8]);

    let bar = row![
        title_text,
        iced::widget::space().width(Fill),
        minimize_btn,
        maximize_btn,
        close_btn,
    ]
    .spacing(4)
    .align_y(iced::Center)
    .padding([4, 8]);

    let bar_container = container(bar)
        .width(Fill)
        .style(theme::title_bar_container);

    mouse_area(bar_container)
        .on_press(Message::DragWindow(window_id))
        .into()
}
