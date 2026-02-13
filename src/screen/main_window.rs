use iced::widget::{button, center, column, container, row, rule, text};
use iced::{Center, Fill};

use crate::app::{Message, RecordingState};
use crate::widget::Element;

pub fn view<'a>(
    recording_state: &RecordingState,
    frame_info: Option<(u32, u32)>,
) -> Element<'a, Message> {
    let menu_bar = row![
        button("Preview").on_press(Message::OpenPreview),
        button("Settings").on_press(Message::OpenSettings),
    ]
    .spacing(5)
    .padding(5);

    let content = {
        let heading = text("Cocuyo").size(28);
        let subtitle = text("Screen capture via PipeWire").size(14);

        let controls: Element<'_, Message> = match recording_state {
            RecordingState::Idle => column![
                button("Start Recording").on_press(Message::StartRecording),
            ]
            .align_x(Center)
            .into(),
            RecordingState::Starting => column![text("Requesting screen capture..."),]
                .align_x(Center)
                .into(),
            RecordingState::Recording => column![
                text("Recording in progress"),
                button("Stop Recording").on_press(Message::StopRecording),
            ]
            .spacing(10)
            .align_x(Center)
            .into(),
            RecordingState::Error(msg) => column![
                text(format!("Error: {}", msg)),
                button("Retry").on_press(Message::StartRecording),
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
        RecordingState::Idle => text("Idle"),
        RecordingState::Starting => text("Starting..."),
        RecordingState::Recording => text("Recording"),
        RecordingState::Error(_) => text("Error"),
    };

    let status_bar = {
        let mut r = row![text("Status: "), status_text].spacing(5);
        if let Some((w, h)) = frame_info {
            r = r.push(text(" | "));
            r = r.push(text(format!("{}x{}", w, h)));
        }
        container(r).padding(5)
    };

    column![menu_bar, rule::horizontal(1), content, rule::horizontal(1), status_bar]
        .width(Fill)
        .height(Fill)
        .into()
}
