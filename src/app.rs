use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use pipewire::spa;
use tracing::warn;

use crate::gst_pipeline::GpuBackend;
use crate::ui::custom_window_frame;

#[derive(Clone, PartialEq)]
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

pub struct CocuyoApp {
    frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<FrameData>>>,
    current_frame: Option<FrameData>,
    texture: Option<egui::TextureHandle>,
    screen_window_open: bool,
    info_window_open: bool,
    content_rect: Option<egui::Rect>,
    recording_state: Arc<Mutex<RecordingState>>,
    start_recording_tx: std::sync::mpsc::Sender<((), GpuBackend)>,
    stop_flag: Arc<AtomicBool>,
    available_backends: Vec<GpuBackend>,
    selected_backend_index: usize,
}

impl CocuyoApp {
    pub fn new(
        frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<FrameData>>>,
        recording_state: Arc<Mutex<RecordingState>>,
        start_recording_tx: std::sync::mpsc::Sender<((), GpuBackend)>,
        stop_flag: Arc<AtomicBool>,
        available_backends: Vec<GpuBackend>,
    ) -> Self {
        Self {
            frame_receiver,
            current_frame: None,
            texture: None,
            screen_window_open: false,
            info_window_open: false,
            content_rect: None,
            recording_state,
            start_recording_tx,
            stop_flag,
            available_backends,
            selected_backend_index: 0,
        }
    }

    fn update_frame(&mut self) {
        let mut receiver = self.frame_receiver.lock().unwrap();
        while let Ok(frame) = receiver.try_recv() {
            self.current_frame = Some(frame);
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

    fn render_backend_selector(&mut self, ui: &mut egui::Ui, id_suffix: &str) {
        ui.horizontal(|ui| {
            ui.label("GPU Backend:");
            egui::ComboBox::from_id_salt(format!("gpu_backend{}", id_suffix))
                .selected_text(
                    self.available_backends
                        .get(self.selected_backend_index)
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "None".to_string()),
                )
                .show_ui(ui, |ui| {
                    for (i, backend) in self.available_backends.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.selected_backend_index,
                            i,
                            backend.to_string(),
                        );
                    }
                });
        });
    }

    fn start_recording(&self) {
        let backend = self
            .available_backends
            .get(self.selected_backend_index)
            .cloned()
            .unwrap_or(GpuBackend::Cpu);
        let _ = self.start_recording_tx.send(((), backend));
    }

    fn render_main_panel(&mut self, ui: &mut egui::Ui) {
        egui::TopBottomPanel::bottom("bottom_panel").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Status:");
                let state = self.recording_state.lock().unwrap().clone();
                match state {
                    RecordingState::Idle => {
                        ui.colored_label(egui::Color32::GRAY, "[Idle]");
                    }
                    RecordingState::Starting => {
                        ui.colored_label(egui::Color32::YELLOW, "[Starting...]");
                    }
                    RecordingState::Recording => {
                        if self.current_frame.is_some() {
                            ui.colored_label(egui::Color32::GREEN, "[Recording]");
                        } else {
                            ui.colored_label(egui::Color32::YELLOW, "[Waiting for frames...]");
                        }
                    }
                    RecordingState::Error(ref msg) => {
                        ui.colored_label(egui::Color32::RED, format!("[Error: {}]", msg));
                    }
                }

                if let Some(ref frame) = self.current_frame {
                    ui.separator();
                    ui.label(format!(
                        "{}x{} | {:?}",
                        frame.width, frame.height, frame.format
                    ));
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Cocuyo");
                ui.label("Screen capture via PipeWire");
                ui.add_space(20.0);

                let state = self.recording_state.lock().unwrap().clone();
                match state {
                    RecordingState::Idle => {
                        self.render_backend_selector(ui, "");
                        ui.add_space(10.0);
                        if ui.button("Start Recording").clicked() {
                            self.start_recording();
                        }
                    }
                    RecordingState::Starting => {
                        ui.spinner();
                        ui.label("Requesting screen capture...");
                    }
                    RecordingState::Recording => {
                        ui.label("Recording in progress");
                        ui.add_space(10.0);
                        if ui.button("Stop Recording").clicked() {
                            self.stop_flag.store(true, Ordering::SeqCst);
                        }
                    }
                    RecordingState::Error(ref msg) => {
                        ui.colored_label(egui::Color32::RED, format!("Error: {}", msg));
                        ui.add_space(10.0);
                        self.render_backend_selector(ui, "_retry");
                        ui.add_space(5.0);
                        if ui.button("Retry").clicked() {
                            self.start_recording();
                        }
                    }
                }
            });
        });
    }

    fn render_preview_window(&mut self, ctx: &egui::Context) {
        let current_frame = self.current_frame.clone();
        let mut window_open = self.screen_window_open;

        let mut window = egui::Window::new("Screen Preview")
            .open(&mut window_open)
            .default_size([800.0, 600.0])
            .resizable(true);

        if let Some(content_rect) = self.content_rect {
            window = window.constrain_to(content_rect);
        }

        window.show(ctx, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                if let Some(ref frame) = current_frame {
                    if let Some(rgba_data) = self.convert_to_rgba(frame) {
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(
                            [frame.width as usize, frame.height as usize],
                            &rgba_data,
                        );

                        let texture = self.texture.get_or_insert_with(|| {
                            ui.ctx().load_texture(
                                "screen_frame",
                                color_image.clone(),
                                egui::TextureOptions::LINEAR,
                            )
                        });

                        texture.set(color_image, egui::TextureOptions::LINEAR);

                        let available_size = ui.available_size();
                        let aspect_ratio = frame.width as f32 / frame.height as f32;

                        let display_size = if available_size.x / available_size.y > aspect_ratio {
                            egui::vec2(available_size.y * aspect_ratio, available_size.y)
                        } else {
                            egui::vec2(available_size.x, available_size.x / aspect_ratio)
                        };

                        ui.image((texture.id(), display_size));
                    }
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                        ui.label("Waiting for frames...");
                    });
                }
            });
        });

        self.screen_window_open = window_open;
    }

    fn render_info_window(&mut self, ctx: &egui::Context) {
        let current_frame = self.current_frame.clone();
        let mut window_open = self.info_window_open;

        let mut window = egui::Window::new("Stream Information")
            .open(&mut window_open)
            .default_size([350.0, 250.0])
            .resizable(true);

        if let Some(content_rect) = self.content_rect {
            window = window.constrain_to(content_rect);
        }

        window.show(ctx, |ui| {
            ui.heading("Stream Details");
            ui.separator();

            if let Some(ref frame) = current_frame {
                ui.label(format!("Width: {} px", frame.width));
                ui.label(format!("Height: {} px", frame.height));
                ui.label(format!("Format: {:?}", frame.format));
                ui.label(format!("Data size: {} bytes", frame.data.len()));

                let aspect_ratio = frame.width as f32 / frame.height as f32;
                ui.label(format!("Aspect ratio: {:.2}", aspect_ratio));

                ui.add_space(10.0);
                ui.separator();

                ui.label(format!("Total pixels: {}", frame.width * frame.height));
                ui.label(format!(
                    "Bytes per pixel: {:.2}",
                    frame.data.len() as f32 / (frame.width * frame.height) as f32
                ));
            } else {
                ui.label("No frame data available");
                ui.add_space(10.0);
                ui.colored_label(egui::Color32::YELLOW, "Waiting for first frame...");
            }
        });

        self.info_window_open = window_open;
    }
}

impl eframe::App for CocuyoApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_frame();

        let mut screen_window_open = self.screen_window_open;
        let mut info_window_open = self.info_window_open;

        let content_rect = custom_window_frame(
            ctx,
            "Cocuyo",
            &mut screen_window_open,
            &mut info_window_open,
            |ui| self.render_main_panel(ui),
        );

        self.screen_window_open = screen_window_open;
        self.info_window_open = info_window_open;
        self.content_rect = Some(content_rect);

        self.render_preview_window(ctx);
        self.render_info_window(ctx);

        ctx.request_repaint();
    }
}
