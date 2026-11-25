//! Error types produced by the local vision service.
//!
//! Every fallible operation in `hs-vision` returns [`VisionResult`], which wraps
//! the single [`VisionError`] enum. The binary converts terminal errors into a
//! non-zero exit code; the library never panics on configuration, frame or I/O
//! failures.

use thiserror::Error;

/// Convenient result alias used across `hs-vision`.
pub type VisionResult<T> = Result<T, VisionError>;

/// Errors produced by the vision pipeline.
///
/// The enum is `#[non_exhaustive]` so new failure modes (for example additional
/// capture backends) can be added without breaking downstream matches.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VisionError {
    /// A frame buffer does not satisfy the [`crate::Frame`] invariants.
    #[error("invalid frame: {0}")]
    InvalidFrame(String),

    /// A gaze point or wire event is malformed (non-finite or out of range).
    #[error("invalid event: {0}")]
    InvalidEvent(String),

    /// The service configuration is semantically invalid.
    #[error("invalid configuration: {0}")]
    Config(String),

    /// The frame source failed while producing a frame.
    #[error("frame source error: {0}")]
    Source(String),

    /// The frame source has no more frames (used by finite sources and tests).
    #[error("frame source exhausted")]
    SourceExhausted,

    /// The camera or Unix socket is not available on this host or platform.
    #[error("unavailable: {0}")]
    Unavailable(String),

    /// Publishing a vision event failed.
    #[error("event sink error: {0}")]
    Sink(String),

    /// JSON encoding or decoding of a vision event failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Underlying filesystem, socket or device I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_display_human_readable_messages() {
        let error = VisionError::InvalidFrame("bad size".to_string());
        assert_eq!(error.to_string(), "invalid frame: bad size");

        let error = VisionError::SourceExhausted;
        assert_eq!(error.to_string(), "frame source exhausted");
    }

    #[test]
    fn io_errors_convert_into_vision_errors() {
        let error: VisionError =
            std::io::Error::new(std::io::ErrorKind::NotFound, "missing").into();
        assert!(matches!(error, VisionError::Io(_)));
    }
}
