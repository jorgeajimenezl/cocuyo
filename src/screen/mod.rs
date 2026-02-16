pub mod bulb_setup;
pub mod main_window;
pub mod preview;
pub mod settings;
pub mod title_bar;
pub mod video_shader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    Settings,
    Preview,
    BulbSetup,
}
