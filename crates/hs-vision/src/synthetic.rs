//! Deterministic synthetic camera used by demos and tests.
//!
//! The source renders a dark, slightly textured room with a bright rectangular
//! "person" that moves along two sinusoids. It never touches hardware and is
//! fully deterministic: two sources created with the same configuration produce
//! byte-identical frames.
//!
//! Pacing is owned by [`crate::VisionService`]; the source returns frames as
//! fast as they are requested and only advertises a nominal frame rate in its
//! description.

use async_trait::async_trait;

use crate::frame::validate_dimensions;
use crate::{Frame, FrameSource, VisionError, VisionResult};

/// Background luminance of the synthetic room.
const BACKGROUND_LUMA: u8 = 28;
/// Luminance of the synthetic person.
const PERSON_LUMA: u8 = 225;
/// Nominal period of the horizontal sinusoidal movement, in frames.
const X_PERIOD_FRAMES: f64 = 120.0;
/// Nominal period of the vertical sinusoidal movement, in frames.
const Y_PERIOD_FRAMES: f64 = 90.0;

/// Deterministic synthetic frame source.
///
/// The room is empty for the first [`SyntheticSource::WARMUP_FRAMES`] frames so
/// a detector can learn a clean background, then a bright rectangular person
/// walks in and moves along two sinusoids. This mirrors the intended production
/// startup sequence and avoids learning the person into the background model.
///
/// The target is present from that point on unless
/// [`SyntheticSource::with_absent_after`] removes it, which makes it possible to
/// exercise tracking-to-absence transitions without a camera.
#[derive(Debug, Clone)]
pub struct SyntheticSource {
    width: u32,
    height: u32,
    fps: u32,
    frame_index: u64,
    max_frames: Option<u64>,
    absent_after: Option<u64>,
}

impl SyntheticSource {
    /// Default synthetic frame width in pixels.
    pub const DEFAULT_WIDTH: u32 = 160;
    /// Default synthetic frame height in pixels.
    pub const DEFAULT_HEIGHT: u32 = 120;
    /// Default nominal frame rate in frames per second.
    pub const DEFAULT_FPS: u32 = 15;
    /// Number of empty-room frames rendered before the person appears.
    pub const WARMUP_FRAMES: u64 = 5;

    /// Create a synthetic source rendering `width` x `height` frames.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::InvalidFrame`] for zero or oversized dimensions.
    pub fn new(width: u32, height: u32) -> VisionResult<Self> {
        validate_dimensions(width, height)?;
        Ok(Self {
            width,
            height,
            fps: Self::DEFAULT_FPS,
            frame_index: 0,
            max_frames: None,
            absent_after: None,
        })
    }

    /// Set the nominal frame rate advertised in the source description.
    ///
    /// Values below 1 are clamped to 1. Pacing itself is handled by the
    /// service, so this only affects logging and heartbeats.
    #[must_use]
    pub fn with_fps(mut self, fps: u32) -> Self {
        self.fps = fps.max(1);
        self
    }

    /// Stop producing frames after `max_frames` frames.
    ///
    /// The next call to [`FrameSource::next_frame`] then returns
    /// [`VisionError::SourceExhausted`], which lets a service run terminate
    /// cleanly in demos and tests.
    #[must_use]
    pub fn with_max_frames(mut self, max_frames: u64) -> Self {
        self.max_frames = Some(max_frames);
        self
    }

    /// Remove the synthetic person from frame `absent_after` onwards.
    ///
    /// Frames before that index contain the moving target; later frames show
    /// only the static background, which a detector reports as absence. Note
    /// that values lower than [`SyntheticSource::WARMUP_FRAMES`] remove the
    /// target before it ever appears.
    #[must_use]
    pub fn with_absent_after(mut self, absent_after: u64) -> Self {
        self.absent_after = Some(absent_after);
        self
    }

    /// Frame width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Nominal frame rate advertised by the source.
    #[must_use]
    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Number of frames produced so far.
    #[must_use]
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// Normalized center of the synthetic person for `frame_index`.
    ///
    /// Returns `None` while the room is empty (the warmup frames) and after the
    /// person left the scene.
    #[must_use]
    pub fn target_center(&self, frame_index: u64) -> Option<(f32, f32)> {
        if frame_index < Self::WARMUP_FRAMES {
            return None;
        }
        if let Some(absent_after) = self.absent_after {
            if frame_index >= absent_after {
                return None;
            }
        }
        let index = frame_index as f64;
        let x = 0.5 + 0.32 * (std::f64::consts::TAU * index / X_PERIOD_FRAMES).sin();
        let y = 0.5 + 0.20 * (std::f64::consts::TAU * index / Y_PERIOD_FRAMES).sin();
        Some((x as f32, y as f32))
    }

    /// Render the frame for the current index.
    fn render(&self) -> Frame {
        let width = self.width as usize;
        let height = self.height as usize;
        let mut luma = vec![0u8; width.saturating_mul(height)];
        for (y, row) in luma.chunks_exact_mut(width).enumerate() {
            for (x, pixel) in row.iter_mut().enumerate() {
                // Deterministic texture: a slightly varying background so the
                // detector sees more than one constant value.
                *pixel = BACKGROUND_LUMA + (((x + y) % 16) as u8);
            }
        }

        if let Some((center_x, center_y)) = self.target_center(self.frame_index) {
            let person_width = (self.width / 8).max(4) as i64;
            let person_height = (self.height / 3).max(6) as i64;
            let center_pixel_x = (center_x * self.width as f32).round() as i64;
            let center_pixel_y = (center_y * self.height as f32).round() as i64;
            let left = (center_pixel_x - person_width / 2).clamp(0, self.width as i64 - 1);
            let top = (center_pixel_y - person_height / 2).clamp(0, self.height as i64 - 1);
            let right = (left + person_width).min(self.width as i64);
            let bottom = (top + person_height).min(self.height as i64);
            for y in top..bottom {
                for x in left..right {
                    let index = y as usize * width + x as usize;
                    if let Some(pixel) = luma.get_mut(index) {
                        *pixel = PERSON_LUMA;
                    }
                }
            }
        }

        Frame {
            width: self.width,
            height: self.height,
            luma,
        }
    }
}

impl Default for SyntheticSource {
    fn default() -> Self {
        Self {
            width: Self::DEFAULT_WIDTH,
            height: Self::DEFAULT_HEIGHT,
            fps: Self::DEFAULT_FPS,
            frame_index: 0,
            max_frames: None,
            absent_after: None,
        }
    }
}

#[async_trait]
impl FrameSource for SyntheticSource {
    async fn next_frame(&mut self) -> VisionResult<Frame> {
        if let Some(max_frames) = self.max_frames {
            if self.frame_index >= max_frames {
                return Err(VisionError::SourceExhausted);
            }
        }
        let frame = self.render();
        self.frame_index = self.frame_index.saturating_add(1);
        Ok(frame)
    }

    fn description(&self) -> String {
        format!(
            "synthetic camera {}x{} @ {} fps",
            self.width, self.height, self.fps
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::max_luma;

    fn centroid_of_bright_pixels(frame: &Frame) -> Option<(f32, f32)> {
        let width = frame.width as usize;
        let height = frame.height as usize;
        let mut sum_x = 0u64;
        let mut sum_y = 0u64;
        let mut count = 0u64;
        for (y, row) in frame.luma.chunks_exact(width).enumerate() {
            for (x, &value) in row.iter().enumerate() {
                if value > 150 {
                    sum_x += x as u64;
                    sum_y += y as u64;
                    count += 1;
                }
            }
        }
        if count == 0 {
            return None;
        }
        let mean_x = sum_x as f64 / count as f64 + 0.5;
        let mean_y = sum_y as f64 / count as f64 + 0.5;
        Some((
            (mean_x / width as f64) as f32,
            (mean_y / height as f64) as f32,
        ))
    }

    #[test]
    fn default_frame_size_matches_the_demo() {
        let source = SyntheticSource::default();
        assert_eq!(source.width(), SyntheticSource::DEFAULT_WIDTH);
        assert_eq!(source.height(), SyntheticSource::DEFAULT_HEIGHT);
        assert_eq!(source.fps(), SyntheticSource::DEFAULT_FPS);
        assert!(source.description().contains("synthetic camera"));
    }

    #[test]
    fn invalid_dimensions_are_rejected() {
        assert!(SyntheticSource::new(0, 120).is_err());
        assert!(SyntheticSource::new(160, 0).is_err());
        assert!(SyntheticSource::new(10_000, 10_000).is_err());
    }

    #[tokio::test]
    async fn frames_are_deterministic() {
        let mut first = SyntheticSource::default();
        let mut second = SyntheticSource::default();
        for _ in 0..5 {
            let frame_a = first.next_frame().await.expect("frame from first source");
            let frame_b = second.next_frame().await.expect("frame from second source");
            assert_eq!(frame_a, frame_b);
        }
    }

    #[tokio::test]
    async fn first_frames_are_an_empty_room_warmup() {
        let mut source = SyntheticSource::default();
        for index in 0..SyntheticSource::WARMUP_FRAMES {
            let frame = source.next_frame().await.expect("warmup frame");
            assert!(
                max_luma(&frame) < 100,
                "warmup frame {index} unexpectedly contains a bright target"
            );
        }
        let first_target_frame = source.next_frame().await.expect("first target frame");
        assert!(max_luma(&first_target_frame) > 150);
    }

    #[tokio::test]
    async fn bright_target_moves_along_the_expected_sinusoid() {
        let mut source = SyntheticSource::default();
        for index in 0..(SyntheticSource::WARMUP_FRAMES + 60) {
            let frame = source.next_frame().await.expect("frame");
            if index < SyntheticSource::WARMUP_FRAMES {
                assert!(max_luma(&frame) < 100);
                continue;
            }
            let expected = source.target_center(index).expect("target is present");
            let observed = centroid_of_bright_pixels(&frame)
                .unwrap_or_else(|| panic!("frame {index}: bright target missing"));
            assert!(
                (observed.0 - expected.0).abs() < 0.03,
                "frame {index}: x {} vs {}",
                observed.0,
                expected.0
            );
            assert!(
                (observed.1 - expected.1).abs() < 0.03,
                "frame {index}: y {} vs {}",
                observed.1,
                expected.1
            );
        }
    }

    #[test]
    fn target_stays_inside_the_frame() {
        let source = SyntheticSource::default();
        for index in SyntheticSource::WARMUP_FRAMES..360u64 {
            let (x, y) = source.target_center(index).expect("target is present");
            assert!((0.05..=0.95).contains(&x), "x was {x}");
            assert!((0.05..=0.95).contains(&y), "y was {y}");
        }
    }

    #[tokio::test]
    async fn absent_after_removes_the_target() {
        let first_target_frame = SyntheticSource::WARMUP_FRAMES;
        let mut source = SyntheticSource::default().with_absent_after(first_target_frame + 2);
        for _ in 0..SyntheticSource::WARMUP_FRAMES {
            let frame = source.next_frame().await.expect("empty-room frame");
            assert!(max_luma(&frame) < 100);
        }
        for _ in 0..2 {
            let frame = source.next_frame().await.expect("frame with target");
            assert!(max_luma(&frame) > 150);
        }
        for _ in 0..3 {
            let frame = source.next_frame().await.expect("frame without target");
            assert!(max_luma(&frame) < 100);
        }
        assert!(source.target_center(first_target_frame + 2).is_none());
        assert!(source.target_center(first_target_frame).is_some());
    }

    #[tokio::test]
    async fn max_frames_exhausts_the_source() {
        let mut source = SyntheticSource::default().with_max_frames(2);
        assert!(source.next_frame().await.is_ok());
        assert!(source.next_frame().await.is_ok());
        assert!(matches!(
            source.next_frame().await,
            Err(VisionError::SourceExhausted)
        ));
        assert_eq!(source.frame_index(), 2);
    }

    #[test]
    fn frame_rate_is_clamped_to_at_least_one() {
        let source = SyntheticSource::default().with_fps(0);
        assert_eq!(source.fps(), 1);
        assert!(source.description().contains("1 fps"));
    }
}
