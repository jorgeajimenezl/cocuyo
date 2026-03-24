use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Arc;

use ashpd::desktop::{
    PersistMode,
    screencast::{
        CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream as ScreencastStream,
    },
};
use pipewire as pw;
use pw::{properties::properties, spa};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::dmabuf_handler;
use super::gst_pipeline::{self, GpuBackend};
use super::vulkan_dmabuf;
use crate::frame::FrameData;

pub struct UserData {
    pub format: spa::param::video::VideoInfoRaw,
    pub frame_sender: mpsc::Sender<Arc<FrameData>>,
    pub gst_converter: Option<gst_pipeline::GstVideoConverter>,
    pub mainloop: pw::main_loop::MainLoopRc,
    pub selected_backend: GpuBackend,
}

pub async fn open_portal() -> ashpd::Result<(
    ScreencastStream,
    OwnedFd,
    ashpd::desktop::Session<Screencast>,
)> {
    let proxy = Screencast::new().await?;
    let session = proxy.create_session(Default::default()).await?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Hidden)
                .set_sources(SourceType::Monitor | SourceType::Window)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await?;

    let response = proxy.start(&session, None, Default::default()).await?.response()?;
    let stream = response
        .streams()
        .first()
        .expect("no stream found / selected")
        .to_owned();

    let fd = proxy.open_pipe_wire_remote(&session, Default::default()).await?;

    Ok((stream, fd, session))
}

pub fn start_streaming(
    node_id: u32,
    fd: OwnedFd,
    frame_sender: mpsc::Sender<Arc<FrameData>>,
    selected_backend: GpuBackend,
) -> Result<(), pw::Error> {
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_fd_rc(fd, None)?;

    let data = UserData {
        format: Default::default(),
        frame_sender,
        gst_converter: None,
        mainloop: mainloop.clone(),
        selected_backend,
    };

    let stream = pw::stream::StreamRc::new(
        core,
        "video-capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_, _user_data, old, new| {
            debug!(?old, ?new, "Stream state changed");
        })
        .param_changed(on_param_changed)
        .process(on_process)
        .register()?;

    debug!(?stream, "Created stream");

    let serialize = |obj: pw::spa::pod::Object| -> Vec<u8> {
        pw::spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &pw::spa::pod::Value::Object(obj),
        )
        .unwrap()
        .0
        .into_inner()
    };

    // Two params: first advertises DMA-BUF with explicit modifier negotiation (preferred),
    // second is the CPU/GStreamer fallback without modifiers.
    let dmabuf_values = serialize(build_dmabuf_modifier_format_param());
    let cpu_values = serialize(build_stream_params());

    let mut params = [
        spa::pod::Pod::from_bytes(&dmabuf_values).unwrap(),
        spa::pod::Pod::from_bytes(&cpu_values).unwrap(),
    ];

    info!("Attempting to connect with DMA-BUF modifier negotiation");
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
    stream: &pw::stream::Stream,
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

    // Request 3 buffers from PipeWire so the compositor has spare buffers to
    // write to while the GPU is still reading from a previously dequeued one.
    const SPA_PARAM_BUFFERS_BUFFERS: u32 = 1;
    let buffers_obj = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamBuffers,
        pw::spa::param::ParamType::Buffers,
        pw::spa::pod::Property {
            key: SPA_PARAM_BUFFERS_BUFFERS,
            flags: pw::spa::pod::PropertyFlags::empty(),
            value: pw::spa::pod::Value::Int(3),
        },
    );

    let values = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(buffers_obj),
    )
    .unwrap()
    .0
    .into_inner();

    let pod = spa::pod::Pod::from_bytes(&values).unwrap();
    let mut params = [pod];
    if let Err(e) = stream.update_params(&mut params) {
        warn!(error = %e, "Failed to update buffer params");
    }
}

fn on_process(stream: &pw::stream::Stream, user_data: &mut UserData) {
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
    // buffer drops here → pw_stream_queue_buffer
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
        user_data.format.modifier(),
    )
    .ok()?;

    // SPA_VIDEO_FLAG_MODIFIER (1 << 2) indicates the modifier was explicitly negotiated.
    // Without this flag, modifier() returns 0 but the buffer may actually use GPU-native tiling.
    const SPA_VIDEO_FLAG_MODIFIER: u32 = 1 << 2;
    let flags_raw = user_data.format.flags().bits();
    let modifier_negotiated = (flags_raw & SPA_VIDEO_FLAG_MODIFIER) != 0;

    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        info!(
            fd = dmabuf.fd,
            format = ?dmabuf.format,
            stride = dmabuf.stride,
            width = dmabuf.width,
            height = dmabuf.height,
            modifier = format!("0x{:x}", dmabuf.modifier),
            modifier_negotiated,
            flags = format!("0x{:x}", flags_raw),
            "DMA-BUF detected"
        );
    });

    // Check if the format is directly importable into Vulkan and modifier is linear.
    // Only trust the modifier value if it was explicitly negotiated (SPA_VIDEO_FLAG_MODIFIER set).
    // Without explicit negotiation, the buffer may use GPU-native tiling even though modifier==0.
    let modifier_is_linear = modifier_negotiated
        && (dmabuf.modifier == u64::from(drm_fourcc::DrmModifier::Linear)
            || dmabuf.modifier == u64::from(drm_fourcc::DrmModifier::Invalid));
    let is_importable = super::formats::is_importable_format(dmabuf.format);
    let vulkan_available = vulkan_dmabuf::is_dmabuf_import_available();

    if modifier_is_linear && is_importable && vulkan_available {
        // Zero-copy path: dup the fd and send DMA-BUF metadata directly
        let duped_fd = match nix::unistd::dup(dmabuf.fd) {
            Ok(fd) => unsafe { OwnedFd::from_raw_fd(fd) },
            Err(e) => {
                warn!(error = %e, "Failed to dup DMA-BUF fd, falling back to GStreamer");
                return try_process_dmabuf_gstreamer(&dmabuf, user_data);
            }
        };

        static ONCE_ZEROCOPY: std::sync::Once = std::sync::Once::new();
        ONCE_ZEROCOPY.call_once(|| {
            info!(
                format = ?dmabuf.format,
                modifier = format!("0x{:x}", dmabuf.modifier),
                "Using DMA-BUF zero-copy Vulkan import path"
            );
        });

        return Some(FrameData::DmaBuf {
            fd: duped_fd,
            width: dmabuf.width,
            height: dmabuf.height,
            drm_format: dmabuf.format,
            stride: dmabuf.stride,
            offset: dmabuf.offset,
            modifier: dmabuf.modifier,
        });
    }

    // Non-importable format or non-linear modifier: use GStreamer conversion
    static ONCE_GSTREAMER: std::sync::Once = std::sync::Once::new();
    ONCE_GSTREAMER.call_once(|| {
        let reason = if !modifier_negotiated {
            "modifier not explicitly negotiated (may be tiled)"
        } else if !modifier_is_linear {
            "non-linear modifier"
        } else if !is_importable {
            "format not directly importable to Vulkan"
        } else {
            "Vulkan DMA-BUF import previously failed"
        };
        info!(
            format = ?dmabuf.format,
            modifier = format!("0x{:x}", dmabuf.modifier),
            modifier_negotiated,
            reason,
            "Using GStreamer CPU conversion path (DMA-BUF available but not zero-copy eligible)"
        );
    });
    try_process_dmabuf_gstreamer(&dmabuf, user_data)
}

/// Lazily initializes the GStreamer converter on the first frame.
///
/// For `Auto`, `dmabuf_fd` is used to detect the compositor's GPU via
/// `/proc/self/fdinfo/<fd>` and pick the matching backend. For explicit
/// backends the value is used directly.
fn lazy_setup_gst_converter(user_data: &mut UserData, dmabuf_fd: Option<std::os::fd::RawFd>) {
    if user_data.gst_converter.is_some() {
        return;
    }

    let backend = match user_data.selected_backend {
        GpuBackend::Auto => {
            let available = gst_pipeline::detect_available_backends();
            gst_pipeline::resolve_auto_backend(dmabuf_fd, &available)
        }
        ref b => b.clone(),
    };

    let width = user_data.format.size().width;
    let height = user_data.format.size().height;
    let format = user_data.format.format();

    match gst_pipeline::GstVideoConverter::new(width, height, format, backend) {
        Ok(converter) => {
            info!(
                backend = %converter.backend(),
                "GStreamer converter initialized"
            );
            user_data.gst_converter = Some(converter);
        }
        Err(e) => {
            error!(error = %e, "Failed to create GStreamer converter");
        }
    }
}

fn try_process_dmabuf_gstreamer(
    dmabuf: &dmabuf_handler::DmaBufBuffer,
    user_data: &mut UserData,
) -> Option<FrameData> {
    lazy_setup_gst_converter(user_data, Some(dmabuf.fd));

    let buffer_size = (dmabuf.stride * dmabuf.height) as usize;
    let converter = user_data.gst_converter.as_mut()?;

    if let Err(e) = converter.push_dmabuf(dmabuf.fd, buffer_size) {
        error!(error = %e, "Failed to push DMA-BUF to GStreamer");
        return None;
    }

    match converter.pull_bgra_frame() {
        Ok(data) => Some(FrameData::Cpu {
            data: Arc::new(data),
            width: user_data.format.size().width,
            height: user_data.format.size().height,
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
        warn!("Using CPU memory copy path (no DMA-BUF available)");
    });

    lazy_setup_gst_converter(user_data, None);

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
        match converter.pull_bgra_frame() {
            Ok(data) => data,
            Err(e) => {
                error!(error = %e, "Failed to pull BGRA frame");
                return None;
            }
        }
    } else {
        slice.to_vec()
    };

    Some(FrameData::Cpu {
        data: Arc::new(converted_data),
        width: user_data.format.size().width,
        height: user_data.format.size().height,
    })
}

fn send_frame(frame: FrameData, user_data: &UserData) -> bool {
    let frame = Arc::new(frame);
    match user_data.frame_sender.try_send(frame) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => {
            // Backpressure: drop frame
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            // Receiver dropped — stop the mainloop
            user_data.mainloop.quit();
            false
        }
    }
}

/// Builds a format param that advertises DMA-BUF with explicit DRM modifier negotiation.
/// Only includes Vulkan-importable formats; the modifier is locked to DRM_FORMAT_MOD_LINEAR.
/// When PipeWire selects this param, it sets SPA_VIDEO_FLAG_MODIFIER so we can trust the
/// modifier value and take the zero-copy Vulkan import path.
fn build_dmabuf_modifier_format_param() -> pw::spa::pod::Object {
    use pw::spa::pod::{Property, PropertyFlags};
    use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags};

    const DRM_FORMAT_MOD_LINEAR: i64 = 0;

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
        // Only Vulkan-importable formats: BGRx→Xrgb8888, RGBA→Abgr8888, RGBx→Xbgr8888
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            pw::spa::param::video::VideoFormat::BGRx,
            pw::spa::param::video::VideoFormat::BGRx,
            pw::spa::param::video::VideoFormat::RGBA,
            pw::spa::param::video::VideoFormat::RGBx,
        ),
        // Modifier with MANDATORY|DONT_FIXATE so PipeWire does explicit modifier negotiation
        // and sets SPA_VIDEO_FLAG_MODIFIER in the negotiated format.
        Property {
            key: pw::spa::param::format::FormatProperties::VideoModifier.as_raw(),
            flags: PropertyFlags::MANDATORY | PropertyFlags::DONT_FIXATE,
            value: pw::spa::pod::Value::Choice(pw::spa::pod::ChoiceValue::Long(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Enum {
                    default: DRM_FORMAT_MOD_LINEAR,
                    alternatives: vec![DRM_FORMAT_MOD_LINEAR],
                },
            ))),
        },
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
