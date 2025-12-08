use std::os::fd::{IntoRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use ashpd::desktop::{
    screencast::{CursorMode, Screencast, SourceType, Stream as ScreencastStream},
    PersistMode,
};
use eframe::egui;
use gstreamer;
use pipewire as pw;
use pw::{properties::properties, spa};

mod dmabuf_handler;
mod gst_pipeline;

struct UserData {
    format: spa::param::video::VideoInfoRaw,
    frame_sender: tokio::sync::mpsc::UnboundedSender<FrameData>,
    gst_converter: Option<gst_pipeline::GstVideoConverter>,
}

#[derive(Clone)]
struct FrameData {
    data: Vec<u8>,
    width: u32,
    height: u32,
    format: spa::param::video::VideoFormat,
}

struct CocuyoApp {
    frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<FrameData>>>,
    current_frame: Option<FrameData>,
    texture: Option<egui::TextureHandle>,
    screen_window_open: bool,
    info_window_open: bool,
    content_rect: Option<egui::Rect>,
}

impl CocuyoApp {
    fn new(frame_receiver: tokio::sync::mpsc::UnboundedReceiver<FrameData>) -> Self {
        Self {
            frame_receiver: Arc::new(Mutex::new(frame_receiver)),
            current_frame: None,
            texture: None,
            screen_window_open: true,
            info_window_open: true,
            content_rect: None,
        }
    }

    fn update_frame(&mut self) {
        let mut receiver = self.frame_receiver.lock().unwrap();
        while let Ok(frame) = receiver.try_recv() {
            self.current_frame = Some(frame);
        }
    }

    fn convert_to_rgba(&self, frame: &FrameData) -> Option<Vec<u8>> {
        // GStreamer handles all format conversions now
        // This method just returns the data which should already be in RGBA format
        if frame.format == spa::param::video::VideoFormat::RGBA {
            Some(frame.data.clone())
        } else {
            eprintln!("Unexpected format: {:?} (should be RGBA from GStreamer)", frame.format);
            None
        }
    }
}

impl eframe::App for CocuyoApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array() // Make sure we don't paint anything behind the rounded corners
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_frame();

        let content_rect = custom_window_frame(ctx, "Cocuyo", |ui| {
            // Bottom panel for status information
            egui::TopBottomPanel::bottom("bottom_panel").show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Status:");
                    if self.current_frame.is_some() {
                        ui.colored_label(egui::Color32::GREEN, "● Streaming");
                    } else {
                        ui.colored_label(egui::Color32::YELLOW, "● Waiting for frames...");
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

            // Central panel
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Cocuyo");
                    ui.label("Screen capture via PipeWire");
                    ui.add_space(20.0);

                    if ui.button("Show Screen Preview").clicked() {
                        self.screen_window_open = true;
                    }

                    ui.add_space(10.0);

                    if ui.button("Show Stream Information").clicked() {
                        self.info_window_open = true;
                    }
                });
            });
        });

        self.content_rect = Some(content_rect);

        // Screen preview window
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

        // Stream information window
        let current_frame_info = self.current_frame.clone();
        let mut info_window_open = self.info_window_open;

        let mut info_window = egui::Window::new("Stream Information")
            .open(&mut info_window_open)
            .default_size([350.0, 250.0])
            .resizable(true);

        if let Some(content_rect) = self.content_rect {
            info_window = info_window.constrain_to(content_rect);
        }

        info_window.show(ctx, |ui| {
                ui.heading("Stream Details");
                ui.separator();

                if let Some(ref frame) = current_frame_info {
                    ui.label(format!("Width: {} px", frame.width));
                    ui.label(format!("Height: {} px", frame.height));
                    ui.label(format!("Format: {:?}", frame.format));
                    ui.label(format!("Data size: {} bytes", frame.data.len()));

                    let aspect_ratio = frame.width as f32 / frame.height as f32;
                    ui.label(format!("Aspect ratio: {:.2}", aspect_ratio));

                    ui.add_space(10.0);
                    ui.separator();

                    ui.label(format!("Total pixels: {}", frame.width * frame.height));
                    ui.label(format!("Bytes per pixel: {:.2}", frame.data.len() as f32 / (frame.width * frame.height) as f32));
                } else {
                    ui.label("No frame data available");
                    ui.add_space(10.0);
                    ui.colored_label(egui::Color32::YELLOW, "Waiting for first frame...");
                }
            });

        self.info_window_open = info_window_open;

        ctx.request_repaint();
    }
}

fn custom_window_frame(ctx: &egui::Context, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) -> egui::Rect {
    use egui::{CentralPanel, UiBuilder};

    let panel_frame = egui::Frame::new()
        .fill(ctx.style().visuals.window_fill())
        .corner_radius(10)
        .stroke(ctx.style().visuals.widgets.noninteractive.fg_stroke)
        .outer_margin(1); // so the stroke is within the bounds

    let mut content_rect_result = egui::Rect::NOTHING;

    CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
        let app_rect = ui.max_rect();

        let title_bar_height = 32.0;
        let title_bar_rect = {
            let mut rect = app_rect;
            rect.max.y = rect.min.y + title_bar_height;
            rect
        };
        title_bar_ui(ui, title_bar_rect, title);

        // Add the contents:
        let content_rect = {
            let mut rect = app_rect;
            rect.min.y = title_bar_rect.max.y;
            rect
        }
        .shrink(4.0);
        content_rect_result = content_rect;
        let mut content_ui = ui.new_child(UiBuilder::new().max_rect(content_rect));
        add_contents(&mut content_ui);
    });

    content_rect_result
}

fn title_bar_ui(ui: &mut egui::Ui, title_bar_rect: eframe::epaint::Rect, title: &str) {
    use egui::{Align2, FontId, Id, PointerButton, Sense, UiBuilder, vec2};

    let painter = ui.painter();

    // Paint the title:
    painter.text(
        title_bar_rect.center(),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(20.0),
        ui.style().visuals.text_color(),
    );

    // Paint the line under the title:
    painter.line_segment(
        [
            title_bar_rect.left_bottom() + vec2(1.0, 0.0),
            title_bar_rect.right_bottom() + vec2(-1.0, 0.0),
        ],
        ui.visuals().widgets.noninteractive.bg_stroke,
    );

    // Interact with the title bar first (for dragging)
    let title_bar_response = ui.interact(
        title_bar_rect,
        Id::new("title_bar_drag"),
        Sense::click_and_drag(),
    );

    // Handle dragging and double-click
    if title_bar_response.double_clicked() {
        let is_maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
    }

    if title_bar_response.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    // Add buttons on top (they will consume clicks on their area)
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(title_bar_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.visuals_mut().button_frame = false;
            ui.add_space(8.0);
            close_maximize_minimize(ui);
        },
    );
}

/// Show some close/maximize/minimize buttons for the native window.
fn close_maximize_minimize(ui: &mut egui::Ui) {
    use egui::{Button, RichText};

    let button_height = 24.0;

    let close_response = ui
        .add(Button::new(RichText::new("❌").size(button_height)))
        .on_hover_text("Close the window");
    if close_response.clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }

    let is_maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
    if is_maximized {
        let maximized_response = ui
            .add(Button::new(RichText::new("🗗").size(button_height)))
            .on_hover_text("Restore window");
        if maximized_response.clicked() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        }
    } else {
        let maximized_response = ui
            .add(Button::new(RichText::new("🗗").size(button_height)))
            .on_hover_text("Maximize window");
        if maximized_response.clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
    }

    let minimized_response = ui
        .add(Button::new(RichText::new("🗕").size(button_height)))
        .on_hover_text("Minimize the window");
    if minimized_response.clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
}

async fn open_portal() -> ashpd::Result<(ScreencastStream, OwnedFd)> {
    let proxy = Screencast::new().await?;
    let session = proxy.create_session().await?;
    proxy
        .select_sources(
            &session,
            CursorMode::Hidden,
            SourceType::Monitor.into(),
            false,
            None,
            PersistMode::DoNot,
        )
        .await?;

    let response = proxy.start(&session, None).await?.response()?;
    let stream = response
        .streams()
        .first()
        .expect("no stream found / selected")
        .to_owned();

    let fd = proxy.open_pipe_wire_remote(&session).await?;

    Ok((stream, fd))
}

fn start_streaming(
    node_id: u32,
    fd: OwnedFd,
    frame_sender: tokio::sync::mpsc::UnboundedSender<FrameData>,
) -> Result<(), pw::Error> {
    pw::init();

    let mainloop = pw::main_loop::MainLoop::new(None)?;
    let context = pw::context::Context::new(&mainloop)?;
    let core = context.connect_fd(fd, None)?;

    let data = UserData {
        format: Default::default(),
        frame_sender,
        gst_converter: None,
    };

    let stream = pw::stream::Stream::new(
        &core,
        "video-capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_, _, old, new| {
            println!("State changed: {:?} -> {:?}", old, new);
        })
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }

            let (media_type, media_subtype) =
                match pw::spa::param::format_utils::parse_format(param) {
                    Ok(v) => v,
                    Err(_) => return,
                };

            if media_type != pw::spa::param::format::MediaType::Video
                || media_subtype != pw::spa::param::format::MediaSubtype::Raw
            {
                return;
            }

            user_data
                .format
                .parse(param)
                .expect("Failed to parse param changed to VideoInfoRaw");

            println!("Got video format:");
            println!(
                "\tformat: {} ({:?})",
                user_data.format.format().as_raw(),
                user_data.format.format()
            );
            println!(
                "\tsize: {}x{}",
                user_data.format.size().width,
                user_data.format.size().height
            );
            println!(
                "\tframerate: {}/{}",
                user_data.format.framerate().num,
                user_data.format.framerate().denom
            );

            // Initialize GStreamer converter with the detected format
            let width = user_data.format.size().width;
            let height = user_data.format.size().height;
            let format = user_data.format.format();

            match gst_pipeline::GstVideoConverter::new(width, height, format) {
                Ok(converter) => {
                    println!("GStreamer converter initialized successfully");
                    user_data.gst_converter = Some(converter);
                }
                Err(e) => {
                    eprintln!("Failed to create GStreamer converter: {}", e);
                }
            }
        })
        .process(|stream, user_data| {
            match stream.dequeue_buffer() {
                None => {
                    eprintln!("Out of buffers");
                }
                Some(mut buffer) => {
                    // Try to extract DMA-BUF first (Phase 3: DMA-BUF detection)
                    match dmabuf_handler::DmaBufBuffer::from_pipewire_buffer(
                        &mut buffer,
                        user_data.format.size().width,
                        user_data.format.size().height,
                        user_data.format.format(),
                    ) {
                        Ok(dmabuf) => {
                            static ONCE: std::sync::Once = std::sync::Once::new();
                            ONCE.call_once(|| {
                                println!("✓ DMA-BUF enabled! fd={}, format={:?}, stride={}, size={}x{}",
                                    dmabuf.fd, dmabuf.format, dmabuf.stride, dmabuf.width, dmabuf.height);
                                println!("  Using DMA-BUF mmap path with GStreamer GPU conversion");
                            });

                            // Map DMA-BUF to read the data
                            let dmabuf_data = unsafe {
                                match dmabuf.map_readonly() {
                                    Ok(data) => data,
                                    Err(e) => {
                                        eprintln!("Failed to map DMA-BUF: {}", e);
                                        return;
                                    }
                                }
                            };

                            // Use GStreamer for format conversion
                            let converted_data = if let Some(ref converter) = user_data.gst_converter {
                                match converter.push_buffer(&dmabuf_data) {
                                    Ok(_) => match converter.pull_rgba_frame() {
                                        Ok(rgba_data) => rgba_data,
                                        Err(e) => {
                                            eprintln!("Failed to convert DMA-BUF frame: {}", e);
                                            return;
                                        }
                                    },
                                    Err(e) => {
                                        eprintln!("Failed to push DMA-BUF to GStreamer: {}", e);
                                        return;
                                    }
                                }
                            } else {
                                eprintln!("No GStreamer converter available");
                                return;
                            };

                            // Send converted frame
                            let frame = FrameData {
                                data: converted_data,
                                width: user_data.format.size().width,
                                height: user_data.format.size().height,
                                format: spa::param::video::VideoFormat::RGBA,
                            };

                            if let Err(e) = user_data.frame_sender.send(frame) {
                                eprintln!("Failed to send frame: {}", e);
                            }
                            return;
                        }
                        Err(e) => {
                            // DMA-BUF not available, use CPU copy path
                            static ONCE: std::sync::Once = std::sync::Once::new();
                            ONCE.call_once(|| {
                                eprintln!("DMA-BUF not available ({}), using CPU copy fallback", e);
                            });
                        }
                    }

                    // CPU copy fallback path (only used if DMA-BUF extraction failed)
                    let datas = buffer.datas_mut();
                    if datas.is_empty() {
                        return;
                    }

                    let data = &mut datas[0];
                    let chunk = data.chunk();
                    let size = chunk.size() as usize;

                    if size == 0 {
                        return;
                    }

                    // Only access data pointer if we're in CPU copy mode (MAP_BUFFERS would have been needed)
                    let data_ptr = match data.data() {
                        Some(ptr) => ptr,
                        None => {
                            eprintln!("Warning: No CPU-mapped data available (this is expected with DMA-BUF mode)");
                            return;
                        }
                    };

                    let slice = unsafe {
                        std::slice::from_raw_parts(
                            data_ptr.as_ptr(),
                            size,
                        )
                    };

                    // Use GStreamer for format conversion if available
                    let converted_data = if let Some(ref converter) = user_data.gst_converter {
                        match converter.push_buffer(slice) {
                            Ok(_) => match converter.pull_rgba_frame() {
                                Ok(rgba_data) => rgba_data,
                                Err(e) => {
                                    eprintln!("Failed to pull RGBA frame: {}", e);
                                    return;
                                }
                            },
                            Err(e) => {
                                eprintln!("Failed to push buffer to GStreamer: {}", e);
                                return;
                            }
                        }
                    } else {
                        // Fallback: send raw data if converter not ready
                        slice.to_vec()
                    };

                    let frame_data = FrameData {
                        data: converted_data,
                        width: user_data.format.size().width,
                        height: user_data.format.size().height,
                        format: if user_data.gst_converter.is_some() {
                            spa::param::video::VideoFormat::RGBA
                        } else {
                            user_data.format.format()
                        },
                    };

                    if user_data.frame_sender.send(frame_data).is_err() {
                        eprintln!("Failed to send frame data");
                    }
                }
            }
        })
        .register()?;

    println!("Created stream {:#?}", stream);

    let obj = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaType,
            Id,
            pw::spa::param::format::MediaType::Video
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaSubtype,
            Id,
            pw::spa::param::format::MediaSubtype::Raw
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            pw::spa::param::video::VideoFormat::RGB,
            pw::spa::param::video::VideoFormat::RGB,
            pw::spa::param::video::VideoFormat::RGBA,
            pw::spa::param::video::VideoFormat::RGBx,
            pw::spa::param::video::VideoFormat::BGRx,
            pw::spa::param::video::VideoFormat::YUY2,
            pw::spa::param::video::VideoFormat::I420,
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            pw::spa::utils::Rectangle {
                width: 320,
                height: 240
            },
            pw::spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            pw::spa::utils::Rectangle {
                width: 4096,
                height: 4096
            }
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            pw::spa::utils::Fraction { num: 25, denom: 1 },
            pw::spa::utils::Fraction { num: 0, denom: 1 },
            pw::spa::utils::Fraction {
                num: 1000,
                denom: 1
            }
        ),
    );
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .unwrap()
    .0
    .into_inner();

    let mut params = [spa::pod::Pod::from_bytes(&values).unwrap()];

    // Phase 3: Try DMA-BUF by not forcing MAP_BUFFERS
    // This allows PipeWire to provide DMA-BUF file descriptors if available
    println!("Attempting to connect with DMA-BUF support...");
    stream.connect(
        spa::utils::Direction::Input,
        Some(node_id),
        pw::stream::StreamFlags::AUTOCONNECT, // Removed MAP_BUFFERS to allow DMA-BUF
        &mut params,
    )?;

    println!("Connected stream (DMA-BUF mode)");

    mainloop.run();

    Ok(())
}

#[tokio::main]
async fn main() {
    // Initialize GStreamer
    gstreamer::init().expect("Failed to initialize GStreamer");
    println!("GStreamer initialized successfully");

    let (stream, fd) = open_portal()
        .await
        .expect("Failed to open portal. Make sure you're running on a Wayland session with XDG Desktop Portal support.");
    let pipewire_node_id = stream.pipe_wire_node_id();

    println!(
        "PipeWire node id: {}, fd: {}",
        pipewire_node_id,
        &fd.try_clone().unwrap().into_raw_fd()
    );

    let (frame_sender, frame_receiver) = tokio::sync::mpsc::unbounded_channel();

    std::thread::spawn(move || {
        if let Err(e) = start_streaming(pipewire_node_id, fd, frame_sender) {
            eprintln!("PipeWire streaming error: {}", e);
        }
    });

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("Cocuyo")
            .with_transparent(true)
            .with_decorations(false),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    println!("Using wgpu backend (Vulkan preferred on Linux for DMA-BUF support)");

    if let Err(e) = eframe::run_native(
        "cocuyo",
        native_options,
        Box::new(|_cc| Ok(Box::new(CocuyoApp::new(frame_receiver)))),
    ) {
        eprintln!("Failed to run eframe application: {}", e);
    }
}
