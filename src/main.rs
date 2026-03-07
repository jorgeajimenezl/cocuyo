#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tracing::info;

mod adapters;
mod ambient;
mod app;
mod config;
mod frame;
mod gpu_context;
mod perf_stats;
mod platform;
mod recording;
mod region;
mod sampling;
mod screen;
mod theme;
mod tray;
mod widget;

use app::Cocuyo;

fn main() -> iced::Result {
    #[cfg(target_os = "windows")]
    unsafe {
        windows::Win32::System::Console::AttachConsole(
            windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
        )
        .ok();
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
                .add_directive("wgpu_hal=warn".parse().unwrap())
                .add_directive("iced_winit=warn".parse().unwrap()),
        )
        .init();

    let app_config = config::AppConfig::load();
    if let Some(ref adapter) = app_config.preferred_adapter {
        // NOTE: This is safe because there are no concurrent threads at this point, and we set the env var before any wgpu code runs.
        unsafe {
            std::env::set_var("WGPU_ADAPTER_NAME", &adapter.name);
            std::env::set_var("WGPU_BACKEND", adapter.backend.to_string());
        };

        info!(adapter = %adapter, "Set WGPU_ADAPTER_NAME and WGPU_BACKEND from config");
    }

    #[cfg(target_os = "linux")]
    {
        gstreamer::init().expect("Failed to initialize GStreamer");
        info!("GStreamer initialized");

        pipewire::init();
    }

    // Create tray on main thread before iced daemon (required by tray-icon on Windows)
    let tray_state = tray::create_tray();

    iced::daemon(
        move || Cocuyo::new(app_config.clone(), tray_state),
        Cocuyo::update,
        Cocuyo::view,
    )
    .title(Cocuyo::title)
    .theme(Cocuyo::theme)
    .style(theme::app_style)
    .subscription(Cocuyo::subscription)
    .font(include_bytes!("../assets/fonts/Geist-Regular.otf").as_slice())
    .default_font(iced::Font::with_name("Geist"))
    .run()
}
