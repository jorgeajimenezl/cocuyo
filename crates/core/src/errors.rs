use thiserror::Error;

/// Error returned when importing a GPU frame into wgpu.
///
/// Wraps a platform-specific boxed error. Construct via [`ImportError::wrap`].
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ImportError(Box<dyn std::error::Error + Send + Sync + 'static>);

impl ImportError {
    /// Wrap any concrete error as an `ImportError`.
    pub fn wrap<E: std::error::Error + Send + Sync + 'static>(e: E) -> Self {
        Self(Box::new(e))
    }
}

/// Error produced during the recording session lifecycle.
#[derive(Debug, Error)]
pub enum RecordingError {
    /// The capture stream ended with an error (e.g. capture thread failed).
    #[error("Capture stream error: {0}")]
    StreamFailed(String),

    /// The capture thread panicked.
    #[error("Capture thread panicked")]
    ThreadPanicked,
}
