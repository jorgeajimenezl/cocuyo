use iced::widget::{button, center, column, container, row, rule, text};
use iced::window;
use iced::{Center, Fill};

use crate::app::Message;
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

pub fn view(
    window_id: window::Id,
    is_ambient_active: bool,
    has_selected_bulbs: bool,
    selected_count: usize,
) -> Element<'static, Message> {
    let menu_bar = container(
        row![
            button("\u{1F4A1} Bulbs")
                .on_press(Message::OpenBulbSetup)
                .style(theme::styled_button),
            button("\u{25B6} Preview")
                .on_press(Message::OpenPreview)
                .style(theme::styled_button),
            button("\u{2699} Settings")
                .on_press(Message::OpenSettings)
                .style(theme::styled_button),
        ]
        .spacing(5)
        .padding(5),
    )
    .width(Fill)
    .style(theme::menu_bar_container);

    let heading = text("Cocuyo")
        .size(28)
        .color(theme::TEXT)
        .font(theme::HEADING_FONT);
    let subtitle = text("WiZ Light Control")
        .size(14)
        .color(theme::TEXT_DIM);

    let header = column![heading, subtitle]
        .spacing(5)
        .align_x(Center);

    let ambient_controls: Element<'static, Message> = if is_ambient_active {
        row![
            text("Ambient active").color(theme::SUCCESS),
            button("Stop Ambient")
                .on_press(Message::StopAmbient)
                .style(theme::styled_button),
        ]
        .spacing(10)
        .align_y(Center)
        .into()
    } else if has_selected_bulbs {
        button("Start Ambient")
            .on_press(Message::StartAmbient)
            .style(theme::styled_button)
            .into()
    } else {
        text("Select bulbs to enable ambient mode")
            .size(14)
            .color(theme::TEXT_DIM)
            .into()
    };

    let ambient_bar = container(ambient_controls)
        .width(Fill)
        .padding(10)
        .center_x(Fill);

    let color_preview_placeholder = center(
        text("Color preview coming soon")
            .size(14)
            .color(theme::TEXT_DIM),
    )
    .width(Fill)
    .height(Fill);

    let status_text = if is_ambient_active {
        text(format!(
            "Ambient active \u{2014} {} bulb{}",
            selected_count,
            if selected_count == 1 { "" } else { "s" }
        ))
        .color(theme::SUCCESS)
    } else if !has_selected_bulbs {
        text("No bulbs selected").color(theme::TEXT_DIM)
    } else {
        text(format!(
            "{} bulb{} selected",
            selected_count,
            if selected_count == 1 { "" } else { "s" }
        ))
        .color(theme::TEXT)
    };
    let status_bar = container(
        row![text("Status: ").color(theme::TEXT_DIM), status_text].spacing(5),
    )
    .padding(5)
    .width(Fill)
    .style(theme::status_bar_container);

    column![
        title_bar::view(window_id, "Cocuyo"),
        rule::horizontal(1).style(theme::styled_rule),
        menu_bar,
        rule::horizontal(1).style(theme::styled_rule),
        container(header).width(Fill).padding(15),
        rule::horizontal(1).style(theme::styled_rule),
        ambient_bar,
        rule::horizontal(1).style(theme::styled_rule),
        color_preview_placeholder,
        rule::horizontal(1).style(theme::styled_rule),
        status_bar,
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
