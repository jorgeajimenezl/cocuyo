use std::fmt;
use std::hash::{Hash, Hasher};

use windows_capture::monitor::Monitor;
use windows_capture::settings::GraphicsCaptureItemType;
use windows_capture::window::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTarget {
    Monitor(Monitor),
    Window(Window),
}

impl Hash for CaptureTarget {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            CaptureTarget::Monitor(m) => {
                0u8.hash(state);
                (m.as_raw_hmonitor() as usize).hash(state);
            }
            CaptureTarget::Window(w) => {
                1u8.hash(state);
                (w.as_raw_hwnd() as usize).hash(state);
            }
        }
    }
}

impl fmt::Display for CaptureTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureTarget::Monitor(m) => {
                let name = m.device_string().unwrap_or_else(|_| "Unknown Monitor".into());
                match (m.width(), m.height()) {
                    (Ok(w), Ok(h)) => write!(f, "{} ({}x{})", name, w, h),
                    _ => write!(f, "{}", name),
                }
            }
            CaptureTarget::Window(w) => {
                let title = w.title().unwrap_or_else(|_| "Untitled".into());
                let process = w.process_name().unwrap_or_else(|_| "unknown".into());
                write!(f, "{} — {}", title, process)
            }
        }
    }
}

impl TryInto<GraphicsCaptureItemType> for CaptureTarget {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_into(self) -> Result<GraphicsCaptureItemType, Self::Error> {
        match self {
            CaptureTarget::Monitor(m) => Ok(m.try_into()?),
            CaptureTarget::Window(w) => Ok(w.try_into()?),
        }
    }
}

/// Tab selection for the picker UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTab {
    Screens,
    Windows,
}

/// Intent that triggered opening the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerIntent {
    StartRecording,
    StartAmbient,
}
