use iced::widget::{
    button, center, checkbox, column, container, row, rule, scrollable, text,
};
use iced::window;
use iced::{Center, Fill};

use crate::app::Message;
use crate::bulb_setup::{BulbSetupMessage, BulbSetupState};
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(
    window_id: window::Id,
    state: &'a BulbSetupState,
) -> Element<'a, Message> {
    let scan_button = if state.is_scanning() {
        button("Scanning...")
            .style(theme::styled_button)
    } else {
        button("\u{27F3} Scan")
            .on_press(Message::BulbSetup(BulbSetupMessage::Scan))
            .style(theme::styled_button)
    };

    let header = row![
        text("Bulb Discovery").size(18).color(theme::TEXT),
        iced::widget::space().width(Fill),
        scan_button,
    ]
    .spacing(15)
    .align_y(Center);

    let bulb_list: Element<'a, Message> = if state.discovered_bulbs().is_empty() {
        center(
            text(if state.is_scanning() {
                "Scanning for bulbs..."
            } else {
                "No bulbs found. Press Scan to discover."
            })
            .size(14)
            .color(theme::TEXT_DIM),
        )
        .into()
    } else {
        let items = state.discovered_bulbs().iter().fold(
            column![].spacing(8).padding(10),
            |col, bulb| {
                let is_selected = state.selected_bulbs().contains(&bulb.mac);
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
                            .on_toggle(move |_| {
                                Message::BulbSetup(BulbSetupMessage::ToggleBulb(mac.clone()))
                            }),
                        text(detail).size(12).color(theme::TEXT_DIM),
                    ]
                    .spacing(10)
                    .align_y(Center),
                )
            },
        );
        scrollable(items).width(Fill).height(Fill).into()
    };

    let selected_count = state.selected_bulbs().len();
    let status_text = if state.is_scanning() {
        text("Scanning...").color(theme::TEXT_DIM)
    } else {
        text(format!(
            "{} bulb{} selected",
            selected_count,
            if selected_count == 1 { "" } else { "s" }
        ))
        .color(theme::TEXT)
    };

    let done_btn = button("Done")
        .on_press(Message::BulbSetup(BulbSetupMessage::Done))
        .style(theme::styled_button);

    let bottom_bar = container(
        row![status_text, iced::widget::space().width(Fill), done_btn]
            .spacing(10)
            .align_y(Center),
    )
    .padding(10)
    .width(Fill)
    .style(theme::status_bar_container);

    column![
        title_bar::view(window_id, "Bulb Setup"),
        rule::horizontal(1).style(theme::styled_rule),
        container(header).width(Fill).padding(15),
        rule::horizontal(1).style(theme::styled_rule),
        bulb_list,
        rule::horizontal(1).style(theme::styled_rule),
        bottom_bar,
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
