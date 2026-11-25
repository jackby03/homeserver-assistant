//! Non-Linux placeholder for the V4L2 capture source.

use std::path::Path;

use async_trait::async_trait;

use crate::{Frame, FrameSource, VisionError, VisionResult};

/// Placeholder for the Linux V4L2 capture source.
///
/// The real implementation lives in `v4l2::linux` and is compiled only on
/// Linux. This stub keeps `hs-vision` buildable on Windows and macOS and always
/// fails with [`VisionError::Unavailable`], so camera usage is a clean runtime
/// error rather than a compile-time platform fork in the binary.
#[derive(Debug, Clone)]
pub struct V4l2Source {
    device: String,
}

impl V4l2Source {
    /// Device node this source would capture from.
    ///
    /// Kept for API parity with the Linux implementation.
    #[must_use]
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Fail because V4L2 capture is only implemented on Linux.
    ///
    /// # Errors
    ///
    /// Always returns [`VisionError::Unavailable`] outside Linux.
    pub fn open(device: impl AsRef<Path>, width: u32, height: u32) -> VisionResult<Self> {
        let _ = (width, height);
        Err(VisionError::Unavailable(format!(
            "V4L2 capture of {} is only available on Linux",
            device.as_ref().display()
        )))
    }
}

#[async_trait]
impl FrameSource for V4l2Source {
    async fn next_frame(&mut self) -> VisionResult<Frame> {
        Err(VisionError::Unavailable(format!(
            "V4L2 capture of {} is only available on Linux",
            self.device
        )))
    }

    fn description(&self) -> String {
        self.device.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_is_unavailable_outside_linux() {
        let error = V4l2Source::open("/dev/video0", 160, 120)
            .expect_err("V4L2 capture must be unavailable outside Linux");
        assert!(matches!(error, VisionError::Unavailable(_)));
        assert!(error.to_string().contains("only available on Linux"));
    }
}
