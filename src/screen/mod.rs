pub mod bulb_setup;
#[cfg(target_os = "windows")]
pub mod capture_picker;
pub mod main_window;
pub mod profile_dialog;
pub mod settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    Settings,
    BulbSetup,
    ProfileDialog,
    #[cfg(target_os = "windows")]
    CapturePicker,
}
