use std::sync::Arc;

use tokio::sync::mpsc;

use crate::app::RecordingState;
use crate::frame::FrameData;

/// Commands sent from the app to the recording subscription.
#[derive(Debug)]
pub enum RecordingCommand {
    Stop,
}

/// Events sent from the recording subscription to the app.
#[derive(Debug, Clone)]
pub enum RecordingEvent {
    /// The subscription is ready and provides a command sender for control.
    Ready(mpsc::Sender<RecordingCommand>),
    StateChanged(RecordingState),
    Frame(Arc<FrameData>),
}

#[cfg(target_os = "linux")]
pub use crate::platform::linux::recording::recording_subscription;
