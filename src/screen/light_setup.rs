use std::collections::HashSet;

use iced::widget::{button, center, checkbox, column, container, row, rule, scrollable, text};
use iced::{Center, Fill, Task};

use crate::config::AppConfig;
use crate::lighting::{LightId, LightInfo};
use crate::theme;

type Element<'a> = iced::Element<'a, Message, iced::Theme, iced::Renderer>;

#[derive(Debug, Clone)]
pub enum Message {
    Scan,
    LightsDiscovered(Vec<LightInfo>),
    ToggleLight(LightId),
    Done,
}

#[derive(Debug, Clone)]
pub enum LightSetupEvent {
    Done,
    SelectionChanged,
    LightsDiscovered,
}

pub struct LightSetupState {
    discovered_lights: Vec<LightInfo>,
    selected_lights: HashSet<String>,
    is_scanning: bool,
}

impl LightSetupState {
    pub fn new(config: &AppConfig) -> Self {
        let saved_lights = config.saved_lights.clone();
        let selected_ids: Vec<String> = config.selected_light_ids.iter().cloned().collect();

        // Only keep selections that correspond to known lights
        let valid_selections: HashSet<String> = selected_ids
            .into_iter()
            .filter(|id| saved_lights.iter().any(|l| l.id.0 == *id))
            .collect();
        Self {
            discovered_lights: saved_lights,
            selected_lights: valid_selections,
            is_scanning: false,
        }
    }

    pub fn update(&mut self, msg: Message) -> (Task<Message>, Option<LightSetupEvent>) {
        match msg {
            Message::Scan => {
                self.is_scanning = true;
                (Task::none(), None)
            }
            Message::LightsDiscovered(new_lights) => {
                self.is_scanning = false;
                for discovered in new_lights {
                    if let Some(existing) = self
                        .discovered_lights
                        .iter_mut()
                        .find(|l| l.id == discovered.id)
                    {
                        existing.backend_data = discovered.backend_data;
                        if discovered.name.is_some() {
                            existing.name = discovered.name;
                        }
                    } else {
                        self.discovered_lights.push(discovered);
                    }
                }
                (Task::none(), Some(LightSetupEvent::LightsDiscovered))
            }
            Message::ToggleLight(id) => {
                if !self.selected_lights.remove(&id.0) {
                    self.selected_lights.insert(id.0);
                }
                (Task::none(), Some(LightSetupEvent::SelectionChanged))
            }
            Message::Done => (Task::none(), Some(LightSetupEvent::Done)),
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
            text("Light Discovery").size(18).color(theme::TEXT),
            iced::widget::space().width(Fill),
            scan_button,
        ]
        .spacing(15)
        .align_y(Center);

        let light_list: Element<'_> = if self.discovered_lights.is_empty() {
            center(
                text(if self.is_scanning {
                    "Scanning for lights..."
                } else {
                    "No lights found. Press Scan to discover."
                })
                .size(14)
                .color(theme::TEXT_DIM),
            )
            .into()
        } else {
            let items =
                self.discovered_lights
                    .iter()
                    .fold(column![].spacing(8).padding(10), |col, light| {
                        let is_selected = self.selected_lights.contains(&light.id.0);
                        let label = light.name.as_deref().unwrap_or("WiZ Bulb").to_string();
                        let detail = match &light.backend_data {
                            crate::lighting::BackendData::Wiz { ip, mac } => {
                                format!("{} - {}", ip, mac)
                            }
                        };
                        let id = light.id.clone();

                        col.push(
                            row![
                                checkbox(is_selected)
                                    .label(label)
                                    .on_toggle(move |_| { Message::ToggleLight(id.clone()) }),
                                text(detail).size(12).color(theme::TEXT_DIM),
                            ]
                            .spacing(10)
                            .align_y(Center),
                        )
                    });
            scrollable(items).width(Fill).height(Fill).into()
        };

        let selected_count = self.selected_lights.len();
        let status_text = if self.is_scanning {
            text("Scanning...").color(theme::TEXT_DIM)
        } else {
            text(format!(
                "{} light{} selected",
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
            light_list,
            rule::horizontal(1).style(theme::styled_rule),
            bottom_bar,
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }

    pub fn discovered_lights(&self) -> &[LightInfo] {
        &self.discovered_lights
    }

    pub fn selected_lights(&self) -> &HashSet<String> {
        &self.selected_lights
    }

    pub fn has_selected_lights(&self) -> bool {
        !self.selected_lights.is_empty()
    }

    pub fn selected_lights_vec(&self) -> Vec<String> {
        self.selected_lights.iter().cloned().collect()
    }

    pub fn selected_light_infos(&self) -> Vec<LightInfo> {
        self.discovered_lights
            .iter()
            .filter(|l| self.selected_lights.contains(&l.id.0))
            .cloned()
            .collect()
    }
}
