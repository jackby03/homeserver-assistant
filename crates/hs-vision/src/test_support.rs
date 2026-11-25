//! Shared fixtures and fakes for the crate's unit tests.
//!
//! This module is compiled only for tests. It provides frame builders and a
//! scripted [`FrameSource`] so detector and service tests stay hermetic: no
//! camera, no filesystem, no network and no sleeps.

use async_trait::async_trait;

use crate::{BoundingBox, Frame, FrameSource, VisionError, VisionResult};

/// Build a uniform grayscale frame.
pub(crate) fn solid_frame(width: u32, height: u32, luma: u8) -> Frame {
    Frame::new(width, height, vec![luma; frame_len(width, height)])
        .expect("test frame dimensions are valid")
}

/// Build a frame with `background` everywhere and a filled `rect` in
/// `foreground`.
pub(crate) fn frame_with_rect(
    width: u32,
    height: u32,
    background: u8,
    rect: BoundingBox,
    foreground: u8,
) -> Frame {
    let mut luma = vec![background; frame_len(width, height)];
    for y in rect.y..rect.y.saturating_add(rect.height) {
        for x in rect.x..rect.x.saturating_add(rect.width) {
            if x < width && y < height {
                let index = y as usize * width as usize + x as usize;
                luma[index] = foreground;
            }
        }
    }
    Frame::new(width, height, luma).expect("test frame dimensions are valid")
}

/// Maximum luminance value in a frame.
pub(crate) fn max_luma(frame: &Frame) -> u8 {
    frame.luma.iter().copied().max().unwrap_or(0)
}

fn frame_len(width: u32, height: u32) -> usize {
    width as usize * height as usize
}

/// A [`FrameSource`] that replays a fixed list of frames.
///
/// Returns [`VisionError::SourceExhausted`] once the script is consumed.
pub(crate) struct ScriptedSource {
    frames: Vec<Frame>,
    next: usize,
    description: String,
}

impl ScriptedSource {
    /// Create a source replaying `frames` in order.
    pub(crate) fn new(frames: Vec<Frame>) -> Self {
        Self {
            frames,
            next: 0,
            description: "scripted test camera".to_string(),
        }
    }
}

#[async_trait]
impl FrameSource for ScriptedSource {
    async fn next_frame(&mut self) -> VisionResult<Frame> {
        let frame = self.frames.get(self.next).cloned();
        match frame {
            Some(frame) => {
                self.next += 1;
                Ok(frame)
            }
            None => Err(VisionError::SourceExhausted),
        }
    }

    fn description(&self) -> String {
        self.description.clone()
    }
}
