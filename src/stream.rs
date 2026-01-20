use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ashpd::desktop::{
    screencast::{CursorMode, Screencast, SourceType, Stream as ScreencastStream},
    PersistMode,
};
use pipewire as pw;
use pw::{properties::properties, spa};
use tracing::{debug, error, info, warn};

use crate::app::FrameData;
use crate::dmabuf_handler;
use crate::gst_pipeline::{self, GpuBackend};

pub struct UserData {
    pub format: spa::param::video::VideoInfoRaw,
    pub frame_sender: tokio::sync::mpsc::UnboundedSender<FrameData>,
    pub gst_converter: Option<gst_pipeline::GstVideoConverter>,
    pub mainloop: pw::main_loop::MainLoop,
    pub selected_backend: GpuBackend,
}

pub async fn open_portal() -> ashpd::Result<(ScreencastStream, OwnedFd)> {
    let proxy = Screencast::new().await?;
    let session = proxy.create_session().await?;
    proxy
        .select_sources(
            &session,
            CursorMode::Hidden,
            (SourceType::Monitor | SourceType::Window).into(),
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

pub fn start_streaming(
    node_id: u32,
    fd: OwnedFd,
    frame_sender: tokio::sync::mpsc::UnboundedSender<FrameData>,
    stop_flag: Arc<AtomicBool>,
    selected_backend: GpuBackend,
) -> Result<(), pw::Error> {
    let mainloop = pw::main_loop::MainLoop::new(None)?;
    let context = pw::context::Context::new(&mainloop)?;
    let core = context.connect_fd(fd, None)?;

    let timer = mainloop.loop_().add_timer(Box::new({
        let stop_flag = stop_flag.clone();
        let mainloop = mainloop.clone();
        move |_| {
            if stop_flag.load(Ordering::SeqCst) {
                info!("Stop requested, quitting mainloop");
                mainloop.quit();
            }
        }
    }));

    timer.update_timer(
        Some(std::time::Duration::from_millis(100)),
        Some(std::time::Duration::from_millis(100)),
    );

    let data = UserData {
        format: Default::default(),
        frame_sender,
        gst_converter: None,
        mainloop: mainloop.clone(),
        selected_backend,
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
            debug!(?old, ?new, "Stream state changed");
        })
        .param_changed(on_param_changed)
        .process(on_process)
        .register()?;

    debug!(?stream, "Created stream");

    let params = build_stream_params();
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(params),
    )
    .unwrap()
    .0
    .into_inner();

    let mut params = [spa::pod::Pod::from_bytes(&values).unwrap()];

    info!("Attempting to connect with DMA-BUF support");
    stream.connect(
        spa::utils::Direction::Input,
        Some(node_id),
        pw::stream::StreamFlags::AUTOCONNECT,
        &mut params,
    )?;

    info!("Connected stream (DMA-BUF mode)");

    mainloop.run();

    Ok(())
}

fn on_param_changed(
    _stream: &pw::stream::StreamRef,
    user_data: &mut UserData,
    id: u32,
    param: Option<&spa::pod::Pod>,
) {
    let Some(param) = param else {
        return;
    };
    if id != pw::spa::param::ParamType::Format.as_raw() {
        return;
    }

    let (media_type, media_subtype) = match pw::spa::param::format_utils::parse_format(param) {
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

    info!(
        format_raw = user_data.format.format().as_raw(),
        format = ?user_data.format.format(),
        width = user_data.format.size().width,
        height = user_data.format.size().height,
        framerate_num = user_data.format.framerate().num,
        framerate_denom = user_data.format.framerate().denom,
        "Got video format"
    );

    let width = user_data.format.size().width;
    let height = user_data.format.size().height;
    let format = user_data.format.format();
    let backend = user_data.selected_backend.clone();

    match gst_pipeline::GstVideoConverter::new(width, height, format, backend) {
        Ok(converter) => {
            info!(
                backend = %converter.backend(),
                "GStreamer converter initialized successfully"
            );
            user_data.gst_converter = Some(converter);
        }
        Err(e) => {
            error!(error = %e, "Failed to create GStreamer converter");
        }
    }
}

fn on_process(stream: &pw::stream::StreamRef, user_data: &mut UserData) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        warn!("Out of buffers");
        return;
    };

    if let Some(frame) = try_process_dmabuf(&mut buffer, user_data) {
        send_frame(frame, user_data);
        return;
    }

    if let Some(frame) = try_process_cpu(&mut buffer, user_data) {
        send_frame(frame, user_data);
    }
}

fn try_process_dmabuf(
    buffer: &mut pw::buffer::Buffer,
    user_data: &mut UserData,
) -> Option<FrameData> {
    let dmabuf = dmabuf_handler::DmaBufBuffer::from_pipewire_buffer(
        buffer,
        user_data.format.size().width,
        user_data.format.size().height,
        user_data.format.format(),
    )
    .ok()?;

    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        info!(
            fd = dmabuf.fd,
            format = ?dmabuf.format,
            stride = dmabuf.stride,
            width = dmabuf.width,
            height = dmabuf.height,
            "DMA-BUF zero-copy enabled"
        );
    });

    let buffer_size = (dmabuf.stride * dmabuf.height) as usize;
    let converter = user_data.gst_converter.as_mut()?;

    if let Err(e) = converter.push_dmabuf(dmabuf.fd, buffer_size) {
        error!(error = %e, "Failed to push DMA-BUF to GStreamer");
        return None;
    }

    match converter.pull_rgba_frame() {
        Ok(data) => Some(FrameData {
            data,
            width: user_data.format.size().width,
            height: user_data.format.size().height,
            format: spa::param::video::VideoFormat::RGBA,
        }),
        Err(e) => {
            error!(error = %e, "Failed to convert DMA-BUF frame");
            None
        }
    }
}

fn try_process_cpu(buffer: &mut pw::buffer::Buffer, user_data: &mut UserData) -> Option<FrameData> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        warn!("DMA-BUF not available, using CPU copy fallback");
    });

    let datas = buffer.datas_mut();
    if datas.is_empty() {
        return None;
    }

    let data = &mut datas[0];
    let chunk = data.chunk();
    let size = chunk.size() as usize;

    if size == 0 {
        return None;
    }

    let data_ptr = data.data()?;
    let slice = unsafe { std::slice::from_raw_parts(data_ptr.as_ptr(), size) };

    let converted_data = if let Some(ref converter) = user_data.gst_converter {
        if let Err(e) = converter.push_buffer(slice) {
            error!(error = %e, "Failed to push buffer to GStreamer");
            return None;
        }
        match converter.pull_rgba_frame() {
            Ok(data) => data,
            Err(e) => {
                error!(error = %e, "Failed to pull RGBA frame");
                return None;
            }
        }
    } else {
        slice.to_vec()
    };

    Some(FrameData {
        data: converted_data,
        width: user_data.format.size().width,
        height: user_data.format.size().height,
        format: if user_data.gst_converter.is_some() {
            spa::param::video::VideoFormat::RGBA
        } else {
            user_data.format.format()
        },
    })
}

fn send_frame(frame: FrameData, user_data: &UserData) {
    if user_data.frame_sender.send(frame).is_err() {
        user_data.mainloop.quit();
    }
}

fn build_stream_params() -> pw::spa::pod::Object {
    pw::spa::pod::object!(
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
    )
}
