use std::os::fd::{IntoRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

use ashpd::desktop::{
    screencast::{CursorMode, Screencast, SourceType, Stream as ScreencastStream},
    PersistMode,
};
use eframe::egui;
use pipewire as pw;
use pw::{properties::properties, spa};

struct UserData {
    format: spa::param::video::VideoInfoRaw,
    frame_sender: tokio::sync::mpsc::UnboundedSender<FrameData>,
}

#[derive(Clone)]
struct FrameData {
    data: Vec<u8>,
    width: u32,
    height: u32,
    format: spa::param::video::VideoFormat,
}

struct ScreenRecorderApp {
    frame_receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<FrameData>>>,
    current_frame: Option<FrameData>,
    texture: Option<egui::TextureHandle>,
}

impl ScreenRecorderApp {
    fn new(frame_receiver: tokio::sync::mpsc::UnboundedReceiver<FrameData>) -> Self {
        Self {
            frame_receiver: Arc::new(Mutex::new(frame_receiver)),
            current_frame: None,
            texture: None,
        }
    }

    fn update_frame(&mut self) {
        let mut receiver = self.frame_receiver.lock().unwrap();
        while let Ok(frame) = receiver.try_recv() {
            self.current_frame = Some(frame);
        }
    }

    fn convert_to_rgba(&self, frame: &FrameData) -> Option<Vec<u8>> {
        match frame.format {
            spa::param::video::VideoFormat::RGB => {
                let mut rgba = Vec::with_capacity((frame.width * frame.height * 4) as usize);
                for chunk in frame.data.chunks(3) {
                    if chunk.len() == 3 {
                        rgba.extend_from_slice(chunk);
                        rgba.push(255);
                    }
                }
                Some(rgba)
            }
            spa::param::video::VideoFormat::RGBA => Some(frame.data.clone()),
            spa::param::video::VideoFormat::RGBx | spa::param::video::VideoFormat::BGRx => {
                let mut rgba = Vec::with_capacity((frame.width * frame.height * 4) as usize);
                for chunk in frame.data.chunks(4) {
                    if chunk.len() == 4 {
                        if frame.format == spa::param::video::VideoFormat::BGRx {
                            rgba.push(chunk[2]);
                            rgba.push(chunk[1]);
                            rgba.push(chunk[0]);
                            rgba.push(255);
                        } else {
                            rgba.push(chunk[0]);
                            rgba.push(chunk[1]);
                            rgba.push(chunk[2]);
                            rgba.push(255);
                        }
                    }
                }
                Some(rgba)
            }
            _ => {
                eprintln!("Unsupported format: {:?}", frame.format);
                None
            }
        }
    }
}

impl eframe::App for ScreenRecorderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_frame();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Screen Recorder");

            if let Some(ref frame) = self.current_frame {
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

                    ui.centered_and_justified(|ui| {
                        ui.image((texture.id(), display_size));
                    });

                    ui.label(format!(
                        "Resolution: {}x{} | Format: {:?}",
                        frame.width, frame.height, frame.format
                    ));
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Waiting for frames...");
                });
            }
        });

        ctx.request_repaint();
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
        })
        .process(|stream, user_data| {
            match stream.dequeue_buffer() {
                None => {
                    eprintln!("Out of buffers");
                }
                Some(mut buffer) => {
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

                    let slice = unsafe {
                        std::slice::from_raw_parts(
                            data.data().unwrap().as_ptr(),
                            size,
                        )
                    };

                    let frame_data = FrameData {
                        data: slice.to_vec(),
                        width: user_data.format.size().width,
                        height: user_data.format.size().height,
                        format: user_data.format.format(),
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

    stream.connect(
        spa::utils::Direction::Input,
        Some(node_id),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;

    println!("Connected stream");

    mainloop.run();

    Ok(())
}

#[tokio::main]
async fn main() {
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
            .with_title("Cocuyo - Screen Recorder"),
        ..Default::default()
    };

    if let Err(e) = eframe::run_native(
        "cocuyo",
        native_options,
        Box::new(|_cc| Ok(Box::new(ScreenRecorderApp::new(frame_receiver)))),
    ) {
        eprintln!("Failed to run eframe application: {}", e);
    }
}
