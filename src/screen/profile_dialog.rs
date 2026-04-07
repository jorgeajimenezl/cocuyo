use iced::widget::{button, column, container, row, rule, scrollable, text, text_input};
use iced::{Fill, Task, padding};

use crate::config::Profile;
use crate::theme;

type Element<'a> = iced::Element<'a, Message, iced::Theme, iced::Renderer>;

#[derive(Debug, Clone)]
pub enum Message {
    NameInputChanged(String),
    Save,
    Load(String),
    Delete(String),
}

#[derive(Debug, Clone)]
pub enum ProfileDialogEvent {
    Save(String),
    Load(String),
    Delete(String),
}

pub struct ProfileDialog {
    name_input: String,
    profiles: Vec<String>,
    active_profile: Option<String>,
    can_save: bool,
}

impl ProfileDialog {
    pub fn new(profiles: &[Profile], active_profile: Option<&str>, can_save: bool) -> Self {
        Self {
            name_input: active_profile.unwrap_or("").to_string(),
            profiles: profiles.iter().map(|p| p.name.clone()).collect(),
            active_profile: active_profile.map(String::from),
            can_save,
        }
    }

    pub fn update(&mut self, message: Message) -> (Task<Message>, Option<ProfileDialogEvent>) {
        match message {
            Message::NameInputChanged(value) => {
                self.name_input = value;
                (Task::none(), None)
            }
            Message::Save => {
                if !self.can_save {
                    return (Task::none(), None);
                }
                let name = self.name_input.trim().to_string();
                if name.is_empty() {
                    return (Task::none(), None);
                }
                if !self.profiles.contains(&name) {
                    self.profiles.push(name.clone());
                }
                self.active_profile = Some(name.clone());
                (Task::none(), Some(ProfileDialogEvent::Save(name)))
            }
            Message::Load(name) => {
                self.name_input = name.clone();
                self.active_profile = Some(name.clone());
                (Task::none(), Some(ProfileDialogEvent::Load(name)))
            }
            Message::Delete(name) => {
                self.profiles.retain(|p| p != &name);
                if self.active_profile.as_deref() == Some(&name) {
                    self.active_profile = None;
                    self.name_input.clear();
                }
                (Task::none(), Some(ProfileDialogEvent::Delete(name)))
            }
        }
    }

    pub fn view(&self) -> Element<'_> {
        let name_input = text_input("Profile name...", &self.name_input)
            .on_input(Message::NameInputChanged)
            .on_submit(Message::Save)
            .size(14)
            .style(theme::styled_text_input);

        let save_enabled = self.can_save && !self.name_input.trim().is_empty();
        let save_btn = if save_enabled {
            button("Save Current Layout")
                .on_press(Message::Save)
                .style(theme::styled_button)
        } else {
            button("Save Current Layout").style(theme::styled_button)
        };

        let hint: Element<'_> = if self.can_save {
            text("Save the current region layout, bulb selection, and ambient settings.")
                .size(12)
                .color(theme::TEXT_DIM)
                .into()
        } else {
            text(
                "Start capture at least once before saving — region coordinates need a frame size.",
            )
            .size(12)
            .color(theme::TEXT_DIM)
            .into()
        };

        let save_section = column![
            text("Save Profile").size(16).color(theme::TEXT),
            hint,
            name_input,
            save_btn,
        ]
        .spacing(8);

        let profiles_section = if self.profiles.is_empty() {
            column![
                text("Saved Profiles").size(16).color(theme::TEXT),
                text("No profiles saved yet.")
                    .size(12)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(8)
        } else {
            let items: Vec<Element<'_>> = self
                .profiles
                .iter()
                .map(|name| {
                    let is_active = self.active_profile.as_deref() == Some(name);
                    let label_color = if is_active {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    };

                    let load_name = name.clone();
                    let delete_name = name.clone();

                    let mut item_row = row![
                        text(name).size(14).color(label_color).width(Fill),
                        button("Load")
                            .on_press(Message::Load(load_name))
                            .style(theme::styled_button)
                            .padding([4, 12]),
                        button("Delete")
                            .on_press(Message::Delete(delete_name))
                            .style(theme::styled_button)
                            .padding([4, 12]),
                    ]
                    .spacing(5)
                    .align_y(iced::Center);

                    if is_active {
                        item_row = item_row.push(text("(active)").size(11).color(theme::TEXT_DIM));
                    }

                    item_row.into()
                })
                .collect();

            column![
                text("Saved Profiles").size(16).color(theme::TEXT),
                scrollable(column(items).spacing(6).width(Fill)).height(Fill),
            ]
            .spacing(8)
        };

        let content = column![
            save_section,
            rule::horizontal(1).style(theme::styled_rule),
            profiles_section,
        ]
        .spacing(16)
        .width(Fill)
        .height(Fill)
        .padding(padding::all(20));

        container(content)
            .width(Fill)
            .height(Fill)
            .style(theme::styled_container)
            .into()
    }
}
