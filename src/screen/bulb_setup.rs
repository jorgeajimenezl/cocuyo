use std::collections::HashSet;

use iced::widget::{button, center, checkbox, column, container, row, rule, scrollable, text};
use iced::{Center, Fill, Task};

use crate::ambient::BulbInfo;
use crate::config::AppConfig;
use crate::theme;

type Element<'a> = iced::Element<'a, Message, iced::Theme, iced::Renderer>;

#[derive(Debug, Clone)]
pub enum Message {
    Scan,
    BulbsDiscovered(Vec<BulbInfo>),
    ToggleBulb(String),
    Done,
}

#[derive(Debug, Clone)]
pub enum BulbSetupEvent {
    Done,
    SelectionChanged,
    BulbsDiscovered,
}

pub struct BulbSetupState {
    discovered_bulbs: Vec<BulbInfo>,
    selected_bulbs: HashSet<String>,
    is_scanning: bool,
}

impl BulbSetupState {
    pub fn new(config: &AppConfig) -> Self {
        let saved_bulbs = config.saved_bulbs.clone();
        let selected_macs: Vec<String> = config.selected_bulb_macs.iter().cloned().collect();

        // Only keep selections that correspond to known bulbs
        let valid_selections: HashSet<String> = selected_macs
            .into_iter()
            .filter(|mac| saved_bulbs.iter().any(|b| b.mac == *mac))
            .collect();
        Self {
            discovered_bulbs: saved_bulbs,
            selected_bulbs: valid_selections,
            is_scanning: false,
        }
    }

    pub fn update(&mut self, msg: Message) -> (Task<Message>, Option<BulbSetupEvent>) {
        match msg {
            Message::Scan => {
                self.is_scanning = true;
                (
                    Task::perform(crate::ambient::discover_bulbs(), Message::BulbsDiscovered),
                    None,
                )
            }
            Message::BulbsDiscovered(new_bulbs) => {
                self.is_scanning = false;
                for discovered in new_bulbs {
                    if let Some(existing) = self
                        .discovered_bulbs
                        .iter_mut()
                        .find(|b| b.mac == discovered.mac)
                    {
                        existing.ip = discovered.ip;
                        if discovered.name.is_some() {
                            existing.name = discovered.name;
                        }
                    } else {
                        self.discovered_bulbs.push(discovered);
                    }
                }
                (Task::none(), Some(BulbSetupEvent::BulbsDiscovered))
            }
            Message::ToggleBulb(mac) => {
                if !self.selected_bulbs.remove(&mac) {
                    self.selected_bulbs.insert(mac);
                }
                (Task::none(), Some(BulbSetupEvent::SelectionChanged))
            }
            Message::Done => (Task::none(), Some(BulbSetupEvent::Done)),
        }
    }

    pub fn view(&self) -> Element<'_> {
        let scan_button = if self.is_scanning {
            button("Scanning...").style(theme::styled_button)
        } else {
            button("\u{27F3} Scan")
                .on_press(Message::Scan)
                .style(theme::styled_button)
        };

        let header = row![
            text("Bulb Discovery").size(18).color(theme::TEXT),
            iced::widget::space().width(Fill),
            scan_button,
        ]
        .spacing(15)
        .align_y(Center);

        let bulb_list: Element<'_> = if self.discovered_bulbs.is_empty() {
            center(
                text(if self.is_scanning {
                    "Scanning for bulbs..."
                } else {
                    "No bulbs found. Press Scan to discover."
                })
                .size(14)
                .color(theme::TEXT_DIM),
            )
            .into()
        } else {
            let items =
                self.discovered_bulbs
                    .iter()
                    .fold(column![].spacing(8).padding(10), |col, bulb| {
                        let is_selected = self.selected_bulbs.contains(&bulb.mac);
                        let label = bulb.name.as_deref().unwrap_or("WiZ Bulb").to_string();
                        let detail = format!("{} - {}", bulb.ip, bulb.mac);
                        let mac = bulb.mac.clone();

                        col.push(
                            row![
                                checkbox(is_selected)
                                    .label(label)
                                    .on_toggle(move |_| { Message::ToggleBulb(mac.clone()) }),
                                text(detail).size(12).color(theme::TEXT_DIM),
                            ]
                            .spacing(10)
                            .align_y(Center),
                        )
                    });
            scrollable(items).width(Fill).height(Fill).into()
        };

        let selected_count = self.selected_bulbs.len();
        let status_text = if self.is_scanning {
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
            .on_press(Message::Done)
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

    pub fn discovered_bulbs(&self) -> &[BulbInfo] {
        &self.discovered_bulbs
    }

    pub fn selected_bulbs(&self) -> &HashSet<String> {
        &self.selected_bulbs
    }

    pub fn has_selected_bulbs(&self) -> bool {
        !self.selected_bulbs.is_empty()
    }

    pub fn set_selected_bulbs(&mut self, macs: impl IntoIterator<Item = String>) {
        self.selected_bulbs = macs
            .into_iter()
            .filter(|m| self.discovered_bulbs.iter().any(|b| b.mac == *m))
            .collect();
    }

    pub fn selected_bulb_infos(&self) -> Vec<BulbInfo> {
        self.discovered_bulbs
            .iter()
            .filter(|b| self.selected_bulbs.contains(&b.mac))
            .cloned()
            .collect()
    }
}
