//! Grayscale frames and the [`FrameSource`] port.
//!
//! The detector only needs luminance: colour conversion happens in the capture
//! source so the hot path works on a compact `u8` plane. Frames live in memory
//! only and are never persisted or transmitted.

use async_trait::async_trait;

use crate::{VisionError, VisionResult};

/// Maximum number of pixels accepted in a [`Frame`].
///
/// A 4096x4096 plane is 16 MiB of luminance, which already exceeds anything a
/// home-server gaze tracker needs. The cap keeps a malformed device format
/// from allocating unbounded memory.
pub const MAX_PIXELS: usize = 16_777_216;

/// A single grayscale frame.
///
/// The fields are public for ergonomic construction in tests and adapters; the
/// canonical way to build a frame is [`Frame::new`], which enforces the
/// invariants: `luma.len() == width * height`, both dimensions non-zero and
/// `width * height <= [`MAX_PIXELS`]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Row-major luminance plane, `0` (black) to `255` (white).
    pub luma: Vec<u8>,
}

impl Frame {
    /// Create a frame after validating its dimensions and buffer length.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::InvalidFrame`] when a dimension is zero, the
    /// pixel count is too large or `luma.len()` does not match the dimensions.
    pub fn new(width: u32, height: u32, luma: Vec<u8>) -> VisionResult<Self> {
        validate_dimensions(width, height)?;
        let expected = (width as usize) * (height as usize);
        if luma.len() != expected {
            return Err(VisionError::InvalidFrame(format!(
                "expected {expected} luma bytes for {width}x{height}, got {}",
                luma.len()
            )));
        }
        Ok(Self {
            width,
            height,
            luma,
        })
    }

    /// Number of luminance bytes in the frame.
    #[must_use]
    pub fn len(&self) -> usize {
        self.luma.len()
    }

    /// Whether the frame carries no pixels.
    ///
    /// A valid frame is never empty; this exists because Clippy expects it next
    /// to [`Frame::len`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.luma.is_empty()
    }

    /// Frame dimensions as `(width, height)`.
    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Read-only view of the luminance plane.
    #[must_use]
    pub fn luma(&self) -> &[u8] {
        &self.luma
    }

    /// Number of pixels in the frame, saturating on exotic platforms.
    #[must_use]
    pub fn pixel_count(&self) -> usize {
        (self.width as usize).saturating_mul(self.height as usize)
    }

    /// Luminance value at `(x, y)`, or `None` when the coordinate is outside
    /// the frame.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.luma
            .get((y as usize) * (self.width as usize) + (x as usize))
            .copied()
    }
}

/// Validate frame dimensions shared by [`Frame::new`] and frame sources.
pub(crate) fn validate_dimensions(width: u32, height: u32) -> VisionResult<()> {
    if width == 0 || height == 0 {
        return Err(VisionError::InvalidFrame(
            "frame dimensions must be non-zero".to_string(),
        ));
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| VisionError::InvalidFrame("frame dimensions overflow".to_string()))?;
    if pixels > MAX_PIXELS {
        return Err(VisionError::InvalidFrame(format!(
            "{width}x{height} exceeds the maximum of {MAX_PIXELS} pixels"
        )));
    }
    Ok(())
}

/// Asynchronous source of grayscale frames.
///
/// Implementations are the camera boundary: a real V4L2 device on Linux, the
/// deterministic [`crate::SyntheticSource`] for demos/tests, or a fake in the
/// daemon. Sources are `Send` because the service may run on any Tokio worker.
#[async_trait]
pub trait FrameSource: Send {
    /// Produce the next frame, waiting for the camera when necessary.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::Unavailable`] when the device is gone, or
    /// [`VisionError::SourceExhausted`] when a finite source has no more frames.
    async fn next_frame(&mut self) -> VisionResult<Frame>;

    /// Short human-readable description used in logs and heartbeats.
    ///
    /// For a real camera this is the device node (for example `/dev/video0`).
    fn description(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_accepts_valid_frame() {
        let frame = Frame::new(4, 3, vec![7; 12]).expect("valid frame");
        assert_eq!(frame.dimensions(), (4, 3));
        assert_eq!(frame.len(), 12);
        assert!(!frame.is_empty());
        assert_eq!(frame.pixel_count(), 12);
        assert_eq!(frame.luma(), &[7; 12]);
    }

    #[test]
    fn new_rejects_zero_dimensions() {
        let error = Frame::new(0, 10, Vec::new()).expect_err("zero width must fail");
        assert!(matches!(error, VisionError::InvalidFrame(_)));
        assert!(Frame::new(10, 0, vec![0; 10]).is_err());
    }

    #[test]
    fn new_rejects_mismatched_buffer_length() {
        let error = Frame::new(4, 3, vec![0; 11]).expect_err("short buffer must fail");
        match error {
            VisionError::InvalidFrame(message) => assert!(message.contains("expected 12")),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn new_rejects_oversized_frames_before_allocating() {
        assert!(Frame::new(8_192, 8_192, Vec::new()).is_err());
        assert!(Frame::new(u32::MAX, u32::MAX, Vec::new()).is_err());
    }

    #[test]
    fn pixel_access_is_bounds_checked() {
        let mut luma = vec![0u8; 6];
        luma[1] = 11;
        luma[4] = 44;
        let frame = Frame::new(3, 2, luma).expect("valid frame");
        assert_eq!(frame.pixel(1, 0), Some(11));
        assert_eq!(frame.pixel(1, 1), Some(44));
        assert_eq!(frame.pixel(3, 0), None);
        assert_eq!(frame.pixel(0, 2), None);
    }
}
