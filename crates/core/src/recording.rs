use std::sync::Arc;

use crate::frame::FrameData;

/// Recording lifecycle state, owned by the application but emitted as part of
/// `RecordingEvent::StateChanged` from the platform recording subscriptions.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordingState {
    Idle,
    Starting,
    Recording,
    Error(String),
}

/// Commands sent from the app to the recording subscription.
#[derive(Debug)]
pub enum RecordingCommand {
    Stop,
}

/// Events sent from the recording subscription to the app.
#[derive(Debug, Clone)]
pub enum RecordingEvent {
    /// The subscription is ready and provides a command sender for control.
    Ready(tokio::sync::mpsc::Sender<RecordingCommand>),
    StateChanged(RecordingState),
    Frame(Arc<FrameData>),
}
