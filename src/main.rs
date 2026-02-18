use tracing::info;

mod ambient;
mod app;
mod bulb_setup;
mod frame;
mod platform;
mod recording;
mod region;
mod sampling;
mod screen;
mod theme;
mod widget;

use app::Cocuyo;
use platform::linux::gst_pipeline::detect_available_backends;

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
        backends = ?available_backends,
        "Detected GPU backends"
    );

    iced::daemon({
        let backends = available_backends;
        move || Cocuyo::new(backends.clone())
    },
        Cocuyo::update,
        Cocuyo::view,
    )
    .title(Cocuyo::title)
    .theme(Cocuyo::theme)
    .style(theme::app_style)
    .subscription(Cocuyo::subscription)
    .font(include_bytes!("../assets/fonts/Geist-Regular.otf").as_slice())
    .font(include_bytes!("../assets/fonts/GeistPixel-Circle.otf").as_slice())
    .default_font(iced::Font::with_name("Geist"))
    .run()
}
