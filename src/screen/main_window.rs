use iced::widget::{button, center, column, container, row, rule, text};
use iced::window;
use iced::{Center, Fill};

use crate::app::{Message, RecordingState};
use crate::screen::title_bar;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(
    window_id: window::Id,
    recording_state: &RecordingState,
    frame_info: Option<(u32, u32)>,
) -> Element<'a, Message> {
    let menu_bar = container(
        row![
            button("Preview")
                .on_press(Message::OpenPreview)
                .style(theme::pixel_button),
            button("Settings")
                .on_press(Message::OpenSettings)
                .style(theme::pixel_button),
        ]
        .spacing(5)
        .padding(5),
    )
    .width(Fill)
    .style(theme::menu_bar_container);

    let content = {
        let heading = text("Cocuyo").size(28).color(theme::GREEN);
        let subtitle = text("Screen capture via PipeWire")
            .size(14)
            .color(theme::TEXT_DIM);

        let controls: Element<'_, Message> = match recording_state {
            RecordingState::Idle => column![
                button("Start Recording")
                    .on_press(Message::StartRecording)
                    .style(theme::pixel_button),
            ]
            .align_x(Center)
            .into(),
            RecordingState::Starting => column![text("Requesting screen capture...")
                .color(theme::YELLOW),]
            .align_x(Center)
            .into(),
            RecordingState::Recording => column![
                text("Recording in progress").color(theme::GREEN),
                button("Stop Recording")
                    .on_press(Message::StopRecording)
                    .style(theme::pixel_button),
            ]
            .spacing(10)
            .align_x(Center)
            .into(),
            RecordingState::Error(msg) => column![
                text(format!("Error: {}", msg)).color(theme::RED),
                button("Retry")
                    .on_press(Message::StartRecording)
                    .style(theme::pixel_button),
            ]
            .spacing(10)
            .align_x(Center)
            .into(),
        };

        center(
            column![heading, subtitle, controls]
                .spacing(15)
                .align_x(Center),
        )
    };

    let status_text = match recording_state {
        RecordingState::Idle => text("Idle").color(theme::TEXT_DIM),
        RecordingState::Starting => text("Starting...").color(theme::YELLOW),
        RecordingState::Recording => text("Recording").color(theme::GREEN),
        RecordingState::Error(_) => text("Error").color(theme::RED),
    };

    let status_bar = {
        let mut r = row![text("Status: ").color(theme::TEXT_DIM), status_text].spacing(5);
        if let Some((w, h)) = frame_info {
            r = r.push(text(" | ").color(theme::TEXT_DIM));
            r = r.push(text(format!("{}x{}", w, h)).color(theme::GREEN));
        }
        container(r).padding(5).width(Fill).style(theme::status_bar_container)
    };

    column![
        title_bar::view(window_id, "Cocuyo"),
        rule::horizontal(1).style(theme::pixel_rule),
        menu_bar,
        rule::horizontal(1).style(theme::pixel_rule),
        content,
        rule::horizontal(1).style(theme::pixel_rule),
        status_bar,
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
