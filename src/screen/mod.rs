#[cfg(target_os = "windows")]
pub mod capture_picker;
pub mod light_setup;
pub mod main_window;
pub mod settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    Settings,
    LightSetup,
    #[cfg(target_os = "windows")]
    CapturePicker,
}
