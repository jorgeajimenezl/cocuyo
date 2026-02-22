use iced::widget::{button, center, column, container, row, rule, scrollable, text};
use iced::window;
use iced::{Center, Fill};

use crate::app::Message;
use crate::platform::windows::capture_target::{CaptureTarget, PickerTab};
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

use windows_capture::monitor::Monitor;
use windows_capture::window::Window;

pub fn view<'a>(
    window_id: window::Id,
    monitors: &[Monitor],
    windows: &[Window],
    selected: Option<&CaptureTarget>,
    active_tab: PickerTab,
) -> Element<'a, Message> {
    // Tab bar
    let screens_tab = button("Screens")
        .on_press(Message::PickerSwitchTab(PickerTab::Screens))
        .style(if active_tab == PickerTab::Screens {
            theme::picker_tab_active
        } else {
            theme::picker_tab
        })
        .padding([4, 16]);

    let windows_tab = button("Windows")
        .on_press(Message::PickerSwitchTab(PickerTab::Windows))
        .style(if active_tab == PickerTab::Windows {
            theme::picker_tab_active
        } else {
            theme::picker_tab
        })
        .padding([4, 16]);

    let tab_bar = container(
        row![screens_tab, windows_tab].spacing(5).padding(8),
    )
    .width(Fill)
    .style(theme::menu_bar_container);

    // List content based on active tab
    let list_area: Element<'a, Message> = match active_tab {
        PickerTab::Screens => {
            if monitors.is_empty() {
                center(
                    text("No monitors found")
                        .size(14)
                        .color(theme::TEXT_DIM),
                )
                .width(Fill)
                .height(Fill)
                .into()
            } else {
                let items = monitors.iter().fold(
                    column![].spacing(2).padding(8),
                    |col, monitor| {
                        let target = CaptureTarget::Monitor(*monitor);
                        let is_selected = selected == Some(&target);

                        let name = monitor
                            .device_string()
                            .unwrap_or_else(|_| "Unknown Monitor".into());
                        let detail = match (monitor.width(), monitor.height()) {
                            (Ok(w), Ok(h)) => format!("{}x{}", w, h),
                            _ => String::new(),
                        };

                        let label_row = row![
                            text(name).size(13).color(theme::TEXT).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                            text(detail).size(12).color(theme::TEXT_DIM),
                        ]
                        .spacing(8)
                        .align_y(Center);

                        col.push(
                            button(label_row)
                                .on_press(Message::PickerSelectTarget(target))
                                .style(if is_selected {
                                    theme::picker_item_selected
                                } else {
                                    theme::picker_item
                                })
                                .width(Fill)
                                .padding([8, 12]),
                        )
                    },
                );
                scrollable(items).width(Fill).height(Fill).into()
            }
        }
        PickerTab::Windows => {
            if windows.is_empty() {
                center(
                    text("No capturable windows found")
                        .size(14)
                        .color(theme::TEXT_DIM),
                )
                .width(Fill)
                .height(Fill)
                .into()
            } else {
                let items = windows.iter().fold(
                    column![].spacing(2).padding(8),
                    |col, window| {
                        let target = CaptureTarget::Window(*window);
                        let is_selected = selected == Some(&target);

                        let title = window
                            .title()
                            .unwrap_or_else(|_| "Untitled".into());
                        let process = window
                            .process_name()
                            .unwrap_or_else(|_| String::new());

                        let label_row = row![
                            text(title).size(13).color(theme::TEXT).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
                            text(process).size(12).color(theme::TEXT_DIM),
                        ]
                        .spacing(8)
                        .align_y(Center);

                        col.push(
                            button(label_row)
                                .on_press(Message::PickerSelectTarget(target))
                                .style(if is_selected {
                                    theme::picker_item_selected
                                } else {
                                    theme::picker_item
                                })
                                .width(Fill)
                                .padding([8, 12]),
                        )
                    },
                );
                scrollable(items).width(Fill).height(Fill).into()
            }
        }
    };

    // Bottom bar
    let status_text = match selected {
        Some(target) => text(format!("{}", target)).size(12).color(theme::TEXT).width(Fill).wrapping(text::Wrapping::WordOrGlyph),
        None => text("Select a target").size(12).color(theme::TEXT_DIM).width(Fill),
    };

    let cancel_btn = button("Cancel")
        .on_press(Message::PickerCancel)
        .style(theme::styled_button);

    let capture_btn = if selected.is_some() {
        button("Capture")
            .on_press(Message::PickerConfirm)
            .style(theme::styled_button)
    } else {
        button("Capture").style(theme::styled_button)
    };

    let bottom_bar = container(
        row![
            status_text,
            cancel_btn,
            capture_btn,
        ]
        .spacing(10)
        .align_y(Center),
    )
    .padding(10)
    .width(Fill)
    .style(theme::status_bar_container);

    column![
        title_bar::view(window_id, "Select Capture Target"),
        rule::horizontal(1).style(theme::styled_rule),
        tab_bar,
        rule::horizontal(1).style(theme::styled_rule),
        list_area,
        rule::horizontal(1).style(theme::styled_rule),
        bottom_bar,
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
