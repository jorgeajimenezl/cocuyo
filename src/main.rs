use tracing::info;

mod adapters;
mod ambient;
mod app;
mod bulb_setup;
mod config;
mod frame;
mod platform;
mod recording;
mod region;
mod sampling;
mod screen;
mod theme;
mod widget;

use app::Cocuyo;

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

    // Apply adapter preference before iced creates its wgpu Instance inside run().
    let app_config = config::AppConfig::load();
    if let Some(ref name) = app_config.preferred_adapter {
        unsafe { std::env::set_var("WGPU_ADAPTER_NAME", name) };
        info!(adapter = %name, "Set WGPU_ADAPTER_NAME from config");
    }

    iced::daemon(
        move || Cocuyo::new(app_config.clone()),
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
