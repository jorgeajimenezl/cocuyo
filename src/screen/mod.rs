pub mod bulb_setup;
#[cfg(target_os = "windows")]
pub mod capture_picker;
pub mod main_window;
pub mod region_overlay;
pub mod settings;
pub mod title_bar;
pub mod video_shader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    Settings,
    BulbSetup,
    #[cfg(target_os = "windows")]
    CapturePicker,
}
