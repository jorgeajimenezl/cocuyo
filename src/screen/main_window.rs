use std::collections::HashSet;

use iced::widget::{button, center, checkbox, column, container, row, rule, scrollable, text};
use iced::window;
use iced::{Center, Fill};

use crate::app::{BulbInfo, Message};
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(
    window_id: window::Id,
    discovered_bulbs: &'a [BulbInfo],
    selected_bulbs: &'a HashSet<String>,
    is_scanning: bool,
) -> Element<'a, Message> {
    let menu_bar = container(
        row![
            button("Preview")
                .on_press(Message::OpenPreview)
                .style(theme::styled_button),
            button("Settings")
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

    let scan_button = if is_scanning {
        button("Scanning...")
            .style(theme::styled_button)
    } else {
        button("Scan")
            .on_press(Message::ScanBulbs)
            .style(theme::styled_button)
    };

    let header = column![heading, subtitle, scan_button]
        .spacing(10)
        .align_x(Center);

    let bulb_list: Element<'a, Message> = if discovered_bulbs.is_empty() {
        center(
            text(if is_scanning {
                "Scanning for bulbs..."
            } else {
                "No bulbs found. Press Scan to discover."
            })
            .size(14)
            .color(theme::TEXT_DIM),
        )
        .into()
    } else {
        let items = discovered_bulbs.iter().fold(
            column![].spacing(8).padding(10),
            |col, bulb| {
                let is_selected = selected_bulbs.contains(&bulb.mac);
                let label = bulb
                    .name
                    .as_deref()
                    .unwrap_or("WiZ Bulb")
                    .to_string();
                let detail = format!("{} - {}", bulb.ip, bulb.mac);
                let mac = bulb.mac.clone();

                col.push(
                    row![
                        checkbox(is_selected)
                            .label(label)
                            .on_toggle(move |_| Message::ToggleBulb(mac.clone())),
                        text(detail).size(12).color(theme::TEXT_DIM),
                    ]
                    .spacing(10)
                    .align_y(Center),
                )
            },
        );
        scrollable(items).width(Fill).height(Fill).into()
    };

    let selected_count = selected_bulbs.len();
    let status_text = if discovered_bulbs.is_empty() {
        text("No bulbs discovered").color(theme::TEXT_DIM)
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
        bulb_list,
        rule::horizontal(1).style(theme::styled_rule),
        status_bar,
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
