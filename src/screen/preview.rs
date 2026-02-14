use iced::widget::{button, center, column, container, row, rule, shader, text};
use iced::window;
use iced::{Center, Fill};

use crate::app::{FrameData, Message, RecordingState};
use crate::screen::title_bar;
use crate::screen::video_shader::VideoScene;
use crate::theme;
use crate::widget::Element;

pub fn view<'a>(
    window_id: window::Id,
    frame: Option<&FrameData>,
    recording_state: &RecordingState,
    frame_info: Option<(u32, u32)>,
) -> Element<'a, Message> {
    let controls: Element<'_, Message> = match recording_state {
        RecordingState::Idle => column![
            button("Start Recording")
                .on_press(Message::StartRecording)
                .style(theme::styled_button),
        ]
        .align_x(Center)
        .into(),
        RecordingState::Starting => column![text("Requesting screen capture...")
            .color(theme::WARNING)]
        .align_x(Center)
        .into(),
        RecordingState::Recording => column![
            text("Recording in progress").color(theme::SUCCESS),
            button("Stop Recording")
                .on_press(Message::StopRecording)
                .style(theme::styled_button),
        ]
        .spacing(10)
        .align_x(Center)
        .into(),
        RecordingState::Error(msg) => column![
            text(format!("Error: {}", msg)).color(theme::DANGER),
            button("Retry")
                .on_press(Message::StartRecording)
                .style(theme::styled_button),
        ]
        .spacing(10)
        .align_x(Center)
        .into(),
    };

    let controls_bar = container(controls)
        .width(Fill)
        .padding(10)
        .center_x(Fill);

    let content: Element<'a, Message> = match frame {
        Some(f) => shader(VideoScene::new(Some(f)))
            .width(Fill)
            .height(Fill)
            .into(),
        None => center(
            column![
                text("Waiting for capture...").size(24).color(theme::TEXT),
                text("Start recording to see the preview")
                    .size(14)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(10)
            .align_x(Center),
        )
        .into(),
    };

    let status_text = match recording_state {
        RecordingState::Idle => text("Idle").color(theme::TEXT_DIM),
        RecordingState::Starting => text("Starting...").color(theme::WARNING),
        RecordingState::Recording => text("Recording").color(theme::SUCCESS),
        RecordingState::Error(_) => text("Error").color(theme::DANGER),
    };

    let status_bar = {
        let mut r = row![text("Status: ").color(theme::TEXT_DIM), status_text].spacing(5);
        if let Some((w, h)) = frame_info {
            r = r.push(text(" | ").color(theme::TEXT_DIM));
            r = r.push(text(format!("{}x{}", w, h)).color(theme::TEXT));
        }
        container(r)
            .padding(5)
            .width(Fill)
            .style(theme::status_bar_container)
    };

    column![
        title_bar::view(window_id, "Preview"),
        rule::horizontal(1).style(theme::styled_rule),
        controls_bar,
        rule::horizontal(1).style(theme::styled_rule),
        content,
        rule::horizontal(1).style(theme::styled_rule),
        status_bar,
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
