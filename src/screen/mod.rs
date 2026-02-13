pub mod main_window;
pub mod preview;
pub mod settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Main,
    Settings,
    Preview,
}
