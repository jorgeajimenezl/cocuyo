use tracing::info;

mod app;
mod dmabuf_handler;
mod formats;
mod gst_pipeline;
mod recording;
mod screen;
mod stream;
mod theme;
mod vulkan_dmabuf;
mod widget;

use app::Cocuyo;
use gst_pipeline::detect_available_backends;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    gstreamer::init().expect("Failed to initialize GStreamer");
    info!("GStreamer initialized");

    pipewire::init();

    let available_backends = detect_available_backends();
    info!(
        backends = ?available_backends.iter().map(|b| b.to_string()).collect::<Vec<_>>(),
        "Detected GPU backends"
    );

    let ab = available_backends.clone();

    iced::daemon(
        move || Cocuyo::new(ab.clone()),
        Cocuyo::update,
        Cocuyo::view,
    )
    .title(Cocuyo::title)
    .theme(Cocuyo::theme)
    .subscription(Cocuyo::subscription)
    .font(include_bytes!("../assets/fonts/Geist-Regular.otf").as_slice())
    .font(include_bytes!("../assets/fonts/GeistPixel-Circle.otf").as_slice())
    .default_font(iced::Font::with_name("Geist"))
    .run()
}
