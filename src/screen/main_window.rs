use iced::widget::{
    button, center, column, container, pick_list, row, rule, scrollable, shader, stack, text,
};
use iced::window;
use iced::{Center, Color, Fill, Length};

use crate::app::{Message, RecordingState};
use crate::config::Profile;
use crate::frame::FrameData;
use crate::perf_stats::PerfStats;
use crate::region::Region;
use crate::sampling;
use crate::theme;
use crate::widget::Element;
use crate::widget::perf_hud::PerfHud;
use crate::widget::region_overlay::RegionOverlay;
use crate::widget::video_shader::VideoScene;

pub fn view<'a>(
    window_id: window::Id,
    frame: Option<&FrameData>,
    recording_state: &RecordingState,
    frame_info: Option<(u32, u32)>,
    is_ambient_active: bool,
    has_selected_bulbs: bool,
    selected_count: usize,
    regions: &'a [Region],
    selected_region: Option<usize>,
    perf_stats: &'a PerfStats,
    show_perf_overlay: bool,
    profiles: &[Profile],
    active_profile_name: Option<&str>,
) -> Element<'a, Message> {
    // Profile dropdown for the menu bar
    let profile_names: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
    let profile_picker: Element<'_, Message> = if profile_names.is_empty() {
        text("No profiles").size(12).color(theme::TEXT_DIM).into()
    } else {
        pick_list(
            active_profile_name.map(String::from),
            profile_names,
            |s: &String| s.clone(),
        )
        .on_select(Message::LoadProfile)
        .text_size(12)
        .style(theme::styled_pick_list)
        .into()
    };

    let menu_bar = container(
        row![
            button("Bulbs")
                .on_press(Message::OpenBulbSetup(window_id))
                .style(theme::styled_button),
            button("Settings")
                .on_press(Message::OpenSettings(window_id))
                .style(theme::styled_button),
            button("Profiles")
                .on_press(Message::OpenProfileDialog(window_id))
                .style(theme::styled_button),
            iced::widget::space().width(Fill),
            profile_picker,
        ]
        .spacing(5)
        .padding(5)
        .align_y(Center),
    )
    .width(Fill)
    .style(theme::menu_bar_container);

    // Left panel: video preview + region overlay + perf HUD
    let preview_area: Element<'a, Message> = match (frame, frame_info) {
        (Some(f), Some((fw, fh))) => {
            let video: Element<'a, Message> =
                shader(VideoScene::new(Some(f))).width(Fill).height(Fill).into();

            let mut layers: Vec<Element<'a, Message>> = vec![video];

            if is_ambient_active {
                let overlay = RegionOverlay::new(regions, fw, fh, selected_region).view();
                layers.push(overlay.into());
            }

            if show_perf_overlay && perf_stats.has_frame_data() {
                let hud: Element<'a, Message> = PerfHud::new(perf_stats).view().into();
                layers.push(hud);
            }

            stack(layers).width(Fill).height(Fill).into()
        }
        _ => center(
            column![
                text("No capture active").size(20).color(theme::TEXT),
                text("Start preview or ambient mode to see the capture")
                    .size(14)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(10)
            .align_x(Center),
        )
        .width(Fill)
        .height(Fill)
        .into(),
    };

    // Right panel: controls
    let ambient_controls: Element<'_, Message> = if is_ambient_active {
        button("Stop Ambient")
            .on_press(Message::StopAmbient)
            .style(theme::styled_button)
            .into()
    } else if has_selected_bulbs {
        button("Start Ambient")
            .on_press(Message::StartAmbient)
            .style(theme::styled_button)
            .into()
    } else {
        text("Select bulbs to enable ambient")
            .size(12)
            .color(theme::TEXT_DIM)
            .into()
    };

    let recording_controls: Element<'_, Message> = if is_ambient_active {
        text("Preview controlled by ambient")
            .size(12)
            .color(theme::WARNING)
            .into()
    } else {
        match recording_state {
            RecordingState::Idle => button("Start Preview")
                .on_press(Message::StartRecording)
                .style(theme::styled_button)
                .into(),
            RecordingState::Starting => text("Starting...").size(12).color(theme::WARNING).into(),
            RecordingState::Recording => column![
                text("Previewing").size(12).color(theme::SUCCESS),
                button("Stop Preview")
                    .on_press(Message::StopRecording)
                    .style(theme::styled_button),
            ]
            .spacing(8)
            .align_x(Center)
            .into(),
            RecordingState::Error(msg) => column![
                text(format!("Error: {}", msg))
                    .size(12)
                    .color(theme::DANGER),
                button("Retry")
                    .on_press(Message::StartRecording)
                    .style(theme::styled_button),
            ]
            .spacing(8)
            .align_x(Center)
            .into(),
        }
    };

    // Region list
    let region_list: Element<'a, Message> = if regions.is_empty() {
        text("Select bulbs to create regions")
            .size(12)
            .color(theme::TEXT_DIM)
            .into()
    } else {
        let items: Vec<Element<'a, Message>> = regions
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let label = format!(
                    "R{} ({})",
                    i + 1,
                    &r.bulb_mac[r.bulb_mac.len().saturating_sub(8)..]
                );

                let color_indicator: Element<'a, Message> = if let Some((cr, cg, cb)) =
                    r.sampled_color
                {
                    container(text(""))
                        .width(14)
                        .height(14)
                        .style(move |_: &_| container::Style {
                            background: Some(iced::Background::Color(Color::from_rgb8(cr, cg, cb))),
                            border: iced::Border {
                                radius: 2.0.into(),
                                width: 1.0,
                                color: theme::BORDER,
                            },
                            ..Default::default()
                        })
                        .into()
                } else {
                    container(text(""))
                        .width(14)
                        .height(14)
                        .style(|_: &_| container::Style {
                            background: Some(iced::Background::Color(theme::BG_SECONDARY)),
                            border: iced::Border {
                                radius: 2.0.into(),
                                width: 1.0,
                                color: theme::BORDER,
                            },
                            ..Default::default()
                        })
                        .into()
                };

                let region_id = r.id;
                let strategy_picker = pick_list(
                    Some(r.strategy.clone()),
                    sampling::all_strategies(),
                    |s: &sampling::BoxedStrategy| s.to_string(),
                )
                .on_select(move |s| Message::RegionStrategyChanged(region_id, s))
                .text_size(11)
                .style(theme::styled_pick_list);

                column![
                    row![color_indicator, text(label).size(12).color(theme::TEXT),]
                        .spacing(5)
                        .align_y(Center),
                    strategy_picker,
                ]
                .spacing(3)
                .into()
            })
            .collect();

        scrollable(column(items).spacing(4).width(Fill))
            .height(Fill)
            .into()
    };

    let mut controls_panel = column![
        text("Controls").size(16).color(theme::TEXT),
        rule::horizontal(1).style(theme::styled_rule),
        ambient_controls,
        rule::horizontal(1).style(theme::styled_rule),
        recording_controls,
        rule::horizontal(1).style(theme::styled_rule),
    ];

    if is_ambient_active {
        controls_panel = controls_panel
            .push(text("Regions").size(14).color(theme::TEXT))
            .push(region_list);
    }

    let controls_panel = controls_panel
        .spacing(8)
        .padding(10)
        .width(Length::Fixed(250.0))
        .height(Fill);

    // Status bar
    let status_text = if is_ambient_active {
        text(format!(
            "Ambient active -- {} bulb{} -- {} region{}",
            selected_count,
            if selected_count == 1 { "" } else { "s" },
            regions.len(),
            if regions.len() == 1 { "" } else { "s" },
        ))
        .color(theme::SUCCESS)
    } else {
        match recording_state {
            RecordingState::Idle => {
                if !has_selected_bulbs {
                    text("No bulbs selected").color(theme::TEXT_DIM)
                } else {
                    text(format!(
                        "{} bulb{} selected",
                        selected_count,
                        if selected_count == 1 { "" } else { "s" }
                    ))
                    .color(theme::TEXT)
                }
            }
            RecordingState::Starting => text("Starting preview...").color(theme::WARNING),
            RecordingState::Recording => text("Previewing").color(theme::SUCCESS),
            RecordingState::Error(_) => text("Error").color(theme::DANGER),
        }
    };

    let mut status_row = row![text("Status: ").color(theme::TEXT_DIM), status_text].spacing(5);
    if let Some((w, h)) = frame_info {
        status_row = status_row
            .push(text(" | ").color(theme::TEXT_DIM))
            .push(text(format!("{}x{}", w, h)).color(theme::TEXT));
    }

    let status_bar = container(status_row)
        .padding(5)
        .width(Fill)
        .style(theme::status_bar_container);

    column![
        menu_bar,
        rule::horizontal(1).style(theme::styled_rule),
        row![
            preview_area,
            rule::vertical(1).style(theme::styled_rule),
            controls_panel,
        ]
        .height(Fill),
        rule::horizontal(1).style(theme::styled_rule),
        status_bar,
    ]
    .width(Fill)
    .height(Fill)
    .into()
}
