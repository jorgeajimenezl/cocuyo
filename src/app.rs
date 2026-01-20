use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use iced::widget::{
    Space, button, center, column, container, image, pick_list, row, rule, scrollable, text,
};
use iced::{Alignment, Color, Element, Length, Size, Subscription, Task, Theme, window};
use pipewire::spa;
use tracing::warn;

use crate::gst_pipeline::GpuBackend;

#[derive(Debug, Clone, PartialEq)]
pub enum RecordingState {
    Idle,
    Starting,
    Recording,
    Error(String),
}

#[derive(Clone)]
pub struct FrameData {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: spa::param::video::VideoFormat,
}

/// Tracks which windows exist
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    Preview,
    StreamInfo,
    Settings,
}

/// All possible messages/events in the application
#[derive(Debug, Clone)]
pub enum Message {
    // Recording control
    StartRecording,
    StopRecording,

    // Window management
    OpenPreview,
    OpenStreamInfo,
    OpenSettings,
    WindowOpened(window::Id, WindowKind),
    WindowClosed(window::Id),

    // Settings
    BackendSelected(GpuBackend),

    // Frame updates
    Tick,
}

pub struct CocuyoApp {
    frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<FrameData>>>,
    current_frame: Option<FrameData>,
    recording_state: Arc<Mutex<RecordingState>>,
    cached_recording_state: RecordingState,
    start_recording_tx: std::sync::mpsc::Sender<((), GpuBackend)>,
    stop_flag: Arc<AtomicBool>,
    available_backends: Vec<GpuBackend>,
    selected_backend: GpuBackend,

    // Window tracking
    windows: BTreeMap<window::Id, WindowKind>,
    main_window_id: Option<window::Id>,
}

impl CocuyoApp {
    pub fn new(
        frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<FrameData>>>,
        recording_state: Arc<Mutex<RecordingState>>,
        start_recording_tx: std::sync::mpsc::Sender<((), GpuBackend)>,
        stop_flag: Arc<AtomicBool>,
        available_backends: Vec<GpuBackend>,
    ) -> (Self, Task<Message>) {
        let selected_backend = available_backends
            .first()
            .cloned()
            .unwrap_or(GpuBackend::Cpu);

        let app = Self {
            frame_receiver,
            current_frame: None,
            recording_state,
            cached_recording_state: RecordingState::Idle,
            start_recording_tx,
            stop_flag,
            available_backends,
            selected_backend,
            windows: BTreeMap::new(),
            main_window_id: None,
        };

        // Open the main window
        let (id, open) = window::open(window::Settings {
            size: Size::new(1280.0, 720.0),
            ..Default::default()
        });

        (
            app,
            open.map(move |_| Message::WindowOpened(id, WindowKind::Main)),
        )
    }

    pub fn title(&self, window_id: window::Id) -> String {
        match self.windows.get(&window_id) {
            Some(WindowKind::Main) => String::from("Cocuyo"),
            Some(WindowKind::Preview) => String::from("Screen Preview"),
            Some(WindowKind::StreamInfo) => String::from("Stream Information"),
            Some(WindowKind::Settings) => String::from("Settings"),
            None => String::from("Cocuyo"),
        }
    }

    fn update_frame(&mut self) {
        if let Ok(mut receiver) = self.frame_receiver.try_lock() {
            while let Ok(frame) = receiver.try_recv() {
                self.current_frame = Some(frame);
            }
        }
    }

    fn update_recording_state(&mut self) {
        if let Ok(state) = self.recording_state.try_lock() {
            self.cached_recording_state = state.clone();
        }
    }

    fn convert_to_rgba(&self, frame: &FrameData) -> Option<Vec<u8>> {
        if frame.format == spa::param::video::VideoFormat::RGBA {
            Some(frame.data.clone())
        } else {
            warn!(format = ?frame.format, "Unexpected format (should be RGBA from GStreamer)");
            None
        }
    }

    fn start_recording(&self) {
        let _ = self
            .start_recording_tx
            .send(((), self.selected_backend.clone()));
    }

    fn has_window(&self, kind: WindowKind) -> bool {
        self.windows.values().any(|k| *k == kind)
    }

    fn find_window_id(&self, kind: WindowKind) -> Option<window::Id> {
        self.windows
            .iter()
            .find(|(_, k)| **k == kind)
            .map(|(id, _)| *id)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::StartRecording => {
                self.start_recording();
                Task::none()
            }
            Message::StopRecording => {
                self.stop_flag.store(true, Ordering::SeqCst);
                Task::none()
            }
            Message::OpenPreview => {
                if self.has_window(WindowKind::Preview) {
                    // Close existing preview window
                    if let Some(id) = self.find_window_id(WindowKind::Preview) {
                        return window::close(id);
                    }
                    Task::none()
                } else {
                    let (id, open) = window::open(window::Settings {
                        size: Size::new(800.0, 600.0),
                        resizable: true,
                        ..Default::default()
                    });
                    open.map(move |_| Message::WindowOpened(id, WindowKind::Preview))
                }
            }
            Message::OpenStreamInfo => {
                if self.has_window(WindowKind::StreamInfo) {
                    if let Some(id) = self.find_window_id(WindowKind::StreamInfo) {
                        return window::close(id);
                    }
                    Task::none()
                } else {
                    let (id, open) = window::open(window::Settings {
                        size: Size::new(350.0, 250.0),
                        resizable: true,
                        ..Default::default()
                    });
                    open.map(move |_| Message::WindowOpened(id, WindowKind::StreamInfo))
                }
            }
            Message::OpenSettings => {
                if self.has_window(WindowKind::Settings) {
                    if let Some(id) = self.find_window_id(WindowKind::Settings) {
                        return window::close(id);
                    }
                    Task::none()
                } else {
                    let (id, open) = window::open(window::Settings {
                        size: Size::new(400.0, 300.0),
                        resizable: true,
                        ..Default::default()
                    });
                    open.map(move |_| Message::WindowOpened(id, WindowKind::Settings))
                }
            }
            Message::WindowOpened(id, kind) => {
                if kind == WindowKind::Main {
                    self.main_window_id = Some(id);
                }
                self.windows.insert(id, kind);
                Task::none()
            }
            Message::WindowClosed(id) => {
                let kind = self.windows.remove(&id);
                // If main window is closed, exit the application
                if kind == Some(WindowKind::Main) {
                    iced::exit()
                } else {
                    Task::none()
                }
            }
            Message::BackendSelected(backend) => {
                self.selected_backend = backend;
                Task::none()
            }
            Message::Tick => {
                self.update_frame();
                self.update_recording_state();
                Task::none()
            }
        }
    }

    pub fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        match self.windows.get(&window_id) {
            Some(WindowKind::Main) => self.view_main_window(),
            Some(WindowKind::Preview) => self.view_preview_window(),
            Some(WindowKind::StreamInfo) => self.view_info_window(),
            Some(WindowKind::Settings) => self.view_settings_window(),
            None => container(text("Loading...")).into(),
        }
    }

    fn view_main_window(&self) -> Element<'_, Message> {
        // Menu bar
        let menu_bar = self.view_menu_bar();

        // Main content
        let main_content = self.view_main_content();

        // Status bar
        let status_bar = self.view_status_bar();

        // Combine all parts
        let content = column![menu_bar, main_content, status_bar]
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(container::rounded_box)
            .into()
    }

    fn view_menu_bar(&self) -> Element<'_, Message> {
        let view_button = button(text("Preview"))
            .on_press(Message::OpenPreview)
            .style(button::text);

        let edit_button = button(text("Settings"))
            .on_press(Message::OpenSettings)
            .style(button::text);

        let stream_info_button = button(text("Info"))
            .on_press(Message::OpenStreamInfo)
            .style(button::text);

        let menu_row = row![
            Space::new().width(8),
            view_button,
            stream_info_button,
            edit_button,
            Space::new().width(Length::Fill),
            text("Cocuyo").size(20),
            Space::new().width(Length::Fill),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .height(32);

        container(menu_row)
            .width(Length::Fill)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(palette.background.weak.color.into()),
                    ..Default::default()
                }
            })
            .into()
    }

    fn view_main_content(&self) -> Element<'_, Message> {
        let title = text("Cocuyo").size(32);
        let subtitle = text("Screen capture via PipeWire").size(14);

        let action_content: Element<Message> = match &self.cached_recording_state {
            RecordingState::Idle => column![
                button(text("Start Recording").size(16))
                    .on_press(Message::StartRecording)
                    .padding([10, 20])
            ]
            .into(),
            RecordingState::Starting => {
                column![text("Requesting screen capture...").size(14),].into()
            }
            RecordingState::Recording => column![
                text("Recording in progress").size(14),
                Space::new().height(10),
                button(text("Stop Recording").size(16))
                    .on_press(Message::StopRecording)
                    .padding([10, 20])
            ]
            .into(),
            RecordingState::Error(msg) => column![
                text(format!("Error: {}", msg))
                    .size(14)
                    .color(Color::from_rgb(0.9, 0.2, 0.2)),
                Space::new().height(10),
                button(text("Retry").size(16))
                    .on_press(Message::StartRecording)
                    .padding([10, 20])
            ]
            .into(),
        };

        let content = column![title, subtitle, Space::new().height(20), action_content,]
            .spacing(8)
            .align_x(Alignment::Center);

        center(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_status_bar(&self) -> Element<'_, Message> {
        let status_indicator: Element<Message> = match &self.cached_recording_state {
            RecordingState::Idle => text("[Idle]")
                .size(12)
                .color(Color::from_rgb(0.5, 0.5, 0.5))
                .into(),
            RecordingState::Starting => text("[Starting...]")
                .size(12)
                .color(Color::from_rgb(0.9, 0.9, 0.2))
                .into(),
            RecordingState::Recording => {
                if self.current_frame.is_some() {
                    text("[Recording]")
                        .size(12)
                        .color(Color::from_rgb(0.2, 0.8, 0.2))
                        .into()
                } else {
                    text("[Waiting for frames...]")
                        .size(12)
                        .color(Color::from_rgb(0.9, 0.9, 0.2))
                        .into()
                }
            }
            RecordingState::Error(msg) => text(format!("[Error: {}]", msg))
                .size(12)
                .color(Color::from_rgb(0.9, 0.2, 0.2))
                .into(),
        };

        let frame_info: Element<Message> = if let Some(ref frame) = self.current_frame {
            text(format!(
                "{}x{} | {:?}",
                frame.width, frame.height, frame.format
            ))
            .size(12)
            .into()
        } else {
            text("").into()
        };

        let status_row = row![
            Space::new().width(8),
            text("Status:").size(12),
            status_indicator,
            Space::new().width(16),
            frame_info,
            Space::new().width(Length::Fill),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .height(24);

        container(status_row)
            .width(Length::Fill)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(palette.background.weak.color.into()),
                    ..Default::default()
                }
            })
            .into()
    }

    fn view_preview_window(&self) -> Element<'_, Message> {
        let content: Element<Message> = if let Some(ref frame) = self.current_frame {
            if let Some(rgba_data) = self.convert_to_rgba(frame) {
                let img_handle = image::Handle::from_rgba(frame.width, frame.height, rgba_data);

                scrollable(
                    container(image(img_handle).content_fit(iced::ContentFit::Contain))
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .center_x(Length::Fill)
                        .center_y(Length::Fill),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            } else {
                center(text("Converting frame..."))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        } else {
            center(text("Waiting for frames..."))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(8)
            .into()
    }

    fn view_info_window(&self) -> Element<'_, Message> {
        let title = text("Stream Details").size(20);

        let details: Element<Message> = if let Some(ref frame) = self.current_frame {
            let aspect_ratio = frame.width as f32 / frame.height as f32;
            let total_pixels = frame.width * frame.height;
            let bytes_per_pixel = frame.data.len() as f32 / total_pixels as f32;

            column![
                text(format!("Width: {} px", frame.width)).size(14),
                text(format!("Height: {} px", frame.height)).size(14),
                text(format!("Format: {:?}", frame.format)).size(14),
                text(format!("Data size: {} bytes", frame.data.len())).size(14),
                text(format!("Aspect ratio: {:.2}", aspect_ratio)).size(14),
                Space::new().height(10),
                rule::horizontal(1),
                Space::new().height(10),
                text(format!("Total pixels: {}", total_pixels)).size(14),
                text(format!("Bytes per pixel: {:.2}", bytes_per_pixel)).size(14),
            ]
            .spacing(4)
            .into()
        } else {
            column![
                text("No frame data available").size(14),
                Space::new().height(10),
                text("Waiting for first frame...")
                    .size(14)
                    .color(Color::from_rgb(0.9, 0.9, 0.2)),
            ]
            .spacing(4)
            .into()
        };

        let content = column![title, rule::horizontal(1), Space::new().height(10), details,]
            .spacing(8)
            .padding(16);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_settings_window(&self) -> Element<'_, Message> {
        let title = text("Settings").size(20);

        let backend_selector = row![
            text("GPU Backend:").size(14),
            pick_list(
                self.available_backends.clone(),
                Some(self.selected_backend.clone()),
                Message::BackendSelected,
            )
            .width(150),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let description = text(
            "Select the GPU backend for video format conversion. \
             Changes take effect on the next recording session.",
        )
        .size(12)
        .color(Color::from_rgb(0.6, 0.6, 0.6));

        let video_section = column![
            text("Video Processing").size(16),
            Space::new().height(8),
            backend_selector,
            Space::new().height(8),
            description,
        ]
        .spacing(4);

        let content = column![
            title,
            rule::horizontal(1),
            Space::new().height(16),
            video_section,
        ]
        .spacing(8)
        .padding(16);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            // Window close events
            window::close_events().map(Message::WindowClosed),
            // Tick for frame updates (60fps)
            iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick),
        ])
    }

    pub fn theme(&self, _window_id: window::Id) -> Theme {
        Theme::Dark
    }
}
