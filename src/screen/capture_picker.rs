use iced::widget::{button, center, column, container, row, rule, scrollable, text};
use iced::{Center, Fill, Task};

use cocuyo_platform_windows::capture_target::{CaptureTarget, PickerIntent, PickerTab};
use crate::theme;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::IsIconic;
use windows_capture::monitor::Monitor;
use windows_capture::window::Window;

type Element<'a> = iced::Element<'a, Message, iced::Theme, iced::Renderer>;

#[derive(Debug, Clone)]
pub enum Message {
    SelectTarget(CaptureTarget),
    SwitchTab(PickerTab),
    Confirm,
    Cancel,
}

#[derive(Debug, Clone)]
pub enum Event {
    TargetSelected(CaptureTarget, PickerIntent),
    Cancelled,
}

/// Returns `true` if the window is actively displayed — not minimized
/// and not cloaked (hidden/suspended by DWM, common with UWP apps).
fn is_window_active(w: &Window) -> bool {
    let hwnd = HWND(w.as_raw_hwnd());

    if unsafe { IsIconic(hwnd) }.as_bool() {
        return false;
    }

    let mut cloaked: u32 = 0;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };

    if result.is_ok() && cloaked != 0 {
        return false;
    }

    true
}

pub struct CapturePicker {
    monitors: Vec<Monitor>,
    windows: Vec<Window>,
    selected: Option<CaptureTarget>,
    active_tab: PickerTab,
    intent: PickerIntent,
}

impl CapturePicker {
    pub fn new(intent: PickerIntent) -> Self {
        let monitors = Monitor::enumerate().unwrap_or_default();
        let windows = Window::enumerate()
            .unwrap_or_default()
            .into_iter()
            .filter(|w| w.title().map(|t| !t.is_empty()).unwrap_or(false))
            .filter(|w| is_window_active(w))
            .collect();

        Self {
            monitors,
            windows,
            selected: None,
            active_tab: PickerTab::Screens,
            intent,
        }
    }

    pub fn update(&mut self, message: Message) -> (Task<Message>, Option<Event>) {
        match message {
            Message::SelectTarget(target) => {
                self.selected = Some(target);
                (Task::none(), None)
            }
            Message::SwitchTab(tab) => {
                self.active_tab = tab;
                (Task::none(), None)
            }
            Message::Confirm => {
                let target = match self.selected.take() {
                    Some(t) => t,
                    None => return (Task::none(), None),
                };
                (
                    Task::none(),
                    Some(Event::TargetSelected(target, self.intent)),
                )
            }
            Message::Cancel => (Task::none(), Some(Event::Cancelled)),
        }
    }

    pub fn view(&self) -> Element<'_> {
        // Tab bar
        let screens_tab = button("Screens")
            .on_press(Message::SwitchTab(PickerTab::Screens))
            .style(if self.active_tab == PickerTab::Screens {
                theme::picker_tab_active
            } else {
                theme::picker_tab
            })
            .padding([4, 16]);

        let windows_tab = button("Windows")
            .on_press(Message::SwitchTab(PickerTab::Windows))
            .style(if self.active_tab == PickerTab::Windows {
                theme::picker_tab_active
            } else {
                theme::picker_tab
            })
            .padding([4, 16]);

        let tab_bar = container(row![screens_tab, windows_tab].spacing(5).padding(8))
            .width(Fill)
            .style(theme::menu_bar_container);

        // List content based on active tab
        let list_area: Element<'_> = match self.active_tab {
            PickerTab::Screens => {
                if self.monitors.is_empty() {
                    center(text("No monitors found").size(14).color(theme::TEXT_DIM))
                        .width(Fill)
                        .height(Fill)
                        .into()
                } else {
                    let items = self.monitors.iter().fold(
                        column![].spacing(2).padding(8),
                        |col, monitor| {
                            let target = CaptureTarget::Monitor(*monitor);
                            let is_selected = self.selected.as_ref() == Some(&target);

                            let name = monitor
                                .device_string()
                                .unwrap_or_else(|_| "Unknown Monitor".into());
                            let detail = match (monitor.width(), monitor.height()) {
                                (Ok(w), Ok(h)) => format!("{}x{}", w, h),
                                _ => String::new(),
                            };

                            let label_row = row![
                                text(name)
                                    .size(13)
                                    .color(theme::TEXT)
                                    .width(Fill)
                                    .wrapping(text::Wrapping::WordOrGlyph),
                                text(detail).size(12).color(theme::TEXT_DIM),
                            ]
                            .spacing(8)
                            .align_y(Center);

                            col.push(
                                button(label_row)
                                    .on_press(Message::SelectTarget(target))
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
                if self.windows.is_empty() {
                    center(
                        text("No capturable windows found")
                            .size(14)
                            .color(theme::TEXT_DIM),
                    )
                    .width(Fill)
                    .height(Fill)
                    .into()
                } else {
                    let items =
                        self.windows
                            .iter()
                            .fold(column![].spacing(2).padding(8), |col, window| {
                                let target = CaptureTarget::Window(*window);
                                let is_selected = self.selected.as_ref() == Some(&target);

                                let title = window.title().unwrap_or_else(|_| "Untitled".into());
                                let process =
                                    window.process_name().unwrap_or_else(|_| String::new());

                                let label_row = row![
                                    text(title)
                                        .size(13)
                                        .color(theme::TEXT)
                                        .width(Fill)
                                        .wrapping(text::Wrapping::WordOrGlyph),
                                    text(process).size(12).color(theme::TEXT_DIM),
                                ]
                                .spacing(8)
                                .align_y(Center);

                                col.push(
                                    button(label_row)
                                        .on_press(Message::SelectTarget(target))
                                        .style(if is_selected {
                                            theme::picker_item_selected
                                        } else {
                                            theme::picker_item
                                        })
                                        .width(Fill)
                                        .padding([8, 12]),
                                )
                            });
                    scrollable(items).width(Fill).height(Fill).into()
                }
            }
        };

        // Bottom bar
        let status_text = match &self.selected {
            Some(target) => text(format!("{}", target))
                .size(12)
                .color(theme::TEXT)
                .width(Fill)
                .wrapping(text::Wrapping::WordOrGlyph),
            None => text("Select a target")
                .size(12)
                .color(theme::TEXT_DIM)
                .width(Fill),
        };

        let cancel_btn = button("Cancel")
            .on_press(Message::Cancel)
            .style(theme::styled_button);

        let capture_btn = if self.selected.is_some() {
            button("Capture")
                .on_press(Message::Confirm)
                .style(theme::styled_button)
        } else {
            button("Capture").style(theme::styled_button)
        };

        let bottom_bar = container(
            row![status_text, cancel_btn, capture_btn,]
                .spacing(10)
                .align_y(Center),
        )
        .padding(10)
        .width(Fill)
        .style(theme::status_bar_container);

        column![
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
}
