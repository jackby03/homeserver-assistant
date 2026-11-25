//! Pure-Rust person detector based on background subtraction.
//!
//! The detector keeps a running-average background model of the scene and flags
//! pixels whose luminance deviates from it by more than
//! [`DetectorConfig::diff_threshold`]. Foreground pixels are aggregated into a
//! centroid and a bounding box; the normalized centroid is smoothed with an
//! exponential moving average so the kiosk avatar does not jitter.
//!
//! No model files, Python or network access are involved: this is a lightweight
//! motion/presence tracker, not a face recognizer. Identity lives in
//! `hs-profiles` and is fed by the optional face-embedding helper.
//!
//! # Assumptions and limitations
//!
//! Background subtraction assumes a mostly static camera and stable lighting.
//! Sudden global illumination changes are rejected as noise (the background is
//! rebuilt instead), while a person standing still is preserved as foreground
//! because foreground pixels do not adapt into the background. Objects that are
//! moved into the scene and left there are reported as foreground until the
//! service restarts.

use crate::frame::Frame;

/// A detection relative to the frame, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundingBox {
    /// Left edge in pixels.
    pub x: u32,
    /// Top edge in pixels.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl BoundingBox {
    /// Number of pixels covered by the box.
    #[must_use]
    pub fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// A single person detection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    /// Smoothed horizontal gaze coordinate, `0.0` left to `1.0` right.
    pub x: f32,
    /// Smoothed vertical gaze coordinate, `0.0` top to `1.0` bottom.
    pub y: f32,
    /// Detector confidence in `[0, 1]`, derived from area and contrast.
    pub confidence: f32,
    /// Axis-aligned bounding box of the foreground region, in pixels.
    pub bbox: BoundingBox,
}

/// Tuning knobs for [`Detector`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorConfig {
    /// Per-pixel absolute luminance difference (in `0..=255`) considered
    /// foreground. Lower values are more sensitive to sensor noise.
    pub diff_threshold: u8,
    /// Exponential adaptation rate for background pixels, `0.001..=0.5`.
    /// Higher values forget the background faster.
    pub learn_rate: f32,
    /// Minimum foreground area, as a fraction of the frame, required to report
    /// a detection. Rejects hot pixels and sensor noise.
    pub min_area_fraction: f32,
    /// Foreground area fraction above which the frame is treated as a global
    /// illumination change: the background is rebuilt and no detection is
    /// reported.
    pub max_area_fraction: f32,
    /// Exponential moving average alpha for the centroid: `1.0` disables
    /// smoothing, smaller values track more slowly and smoothly.
    pub smoothing: f32,
    /// Consecutive misses after which the smoothed centroid is forgotten.
    pub smoothing_reset_frames: u32,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            diff_threshold: 25,
            learn_rate: 0.05,
            min_area_fraction: 0.005,
            max_area_fraction: 0.6,
            smoothing: 0.35,
            smoothing_reset_frames: 3,
        }
    }
}

impl DetectorConfig {
    /// Clamp user-provided values into the supported ranges.
    fn sanitized(self) -> Self {
        let min_area_fraction = self.min_area_fraction.clamp(0.000_5, 0.25);
        Self {
            diff_threshold: self.diff_threshold.max(1),
            learn_rate: self.learn_rate.clamp(0.001, 0.5),
            min_area_fraction,
            max_area_fraction: self.max_area_fraction.clamp(min_area_fraction * 2.0, 0.98),
            smoothing: self.smoothing.clamp(0.01, 1.0),
            smoothing_reset_frames: self.smoothing_reset_frames.max(1),
        }
    }
}

/// Background-subtraction detector with centroid smoothing.
///
/// Create one detector per camera and feed it frames in order with
/// [`Detector::detect`]. The first frame of a stream (and the first frame after
/// a resolution change or a global illumination change) only seeds the
/// background model and returns `None`.
#[derive(Debug)]
pub struct Detector {
    config: DetectorConfig,
    background: Vec<f32>,
    smoothed: Option<(f32, f32)>,
    missed_frames: u32,
}

impl Default for Detector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector {
    /// Create a detector with default [`DetectorConfig`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(DetectorConfig::default())
    }

    /// Create a detector with a custom configuration, clamped to valid ranges.
    #[must_use]
    pub fn with_config(config: DetectorConfig) -> Self {
        Self {
            config: config.sanitized(),
            background: Vec::new(),
            smoothed: None,
            missed_frames: 0,
        }
    }

    /// Effective configuration after clamping.
    #[must_use]
    pub fn config(&self) -> &DetectorConfig {
        &self.config
    }

    /// Forget the background model and the smoothed centroid.
    pub fn reset(&mut self) {
        self.background.clear();
        self.smoothed = None;
        self.missed_frames = 0;
    }

    /// Detect a person in `frame`.
    ///
    /// Returns `None` for the first frame, when the foreground is too small to
    /// be a person, or when a global illumination change was detected.
    #[must_use]
    pub fn detect(&mut self, frame: &Frame) -> Option<Detection> {
        let width = frame.width as usize;
        let height = frame.height as usize;
        if width == 0 || height == 0 {
            return None;
        }
        let total = width.checked_mul(height)?;
        if frame.luma.len() != total {
            self.reset();
            return None;
        }

        if self.background.len() != total {
            self.background.clear();
            self.background
                .extend(frame.luma.iter().map(|&value| f32::from(value)));
            self.smoothed = None;
            self.missed_frames = 0;
            return None;
        }

        let config = self.config;
        let threshold = f32::from(config.diff_threshold);
        let mut area: u64 = 0;
        let mut sum_x: u64 = 0;
        let mut sum_y: u64 = 0;
        let mut diff_sum: f64 = 0.0;
        let mut min_x = usize::MAX;
        let mut min_y = usize::MAX;
        let mut max_x = 0usize;
        let mut max_y = 0usize;

        let rows = frame.luma.chunks_exact(width);
        let background_rows = self.background.chunks_exact_mut(width);
        for (y, (row, background_row)) in rows.zip(background_rows).enumerate() {
            for (x, (&luma, background)) in row.iter().zip(background_row.iter_mut()).enumerate() {
                let difference = (f32::from(luma) - *background).abs();
                if difference > threshold {
                    area += 1;
                    sum_x += x as u64;
                    sum_y += y as u64;
                    diff_sum += f64::from(difference);
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                } else {
                    *background += config.learn_rate * (f32::from(luma) - *background);
                }
            }
        }

        if area == 0 {
            return self.register_miss();
        }

        let area_fraction = area as f32 / total as f32;
        if area_fraction > config.max_area_fraction {
            // Global illumination change (light switched, camera moved): trust
            // the new frame as background instead of reporting a huge blob.
            for (background, &luma) in self.background.iter_mut().zip(frame.luma.iter()) {
                *background = f32::from(luma);
            }
            self.smoothed = None;
            self.missed_frames = 0;
            return None;
        }

        let min_area = (config.min_area_fraction * total as f32).ceil().max(1.0) as u64;
        if area < min_area {
            return self.register_miss();
        }

        let centroid_x = (sum_x as f64 / area as f64 + 0.5) / width as f64;
        let centroid_y = (sum_y as f64 / area as f64 + 0.5) / height as f64;
        let mut x = (centroid_x as f32).clamp(0.0, 1.0);
        let mut y = (centroid_y as f32).clamp(0.0, 1.0);
        if let Some((previous_x, previous_y)) = self.smoothed {
            x = config.smoothing * x + (1.0 - config.smoothing) * previous_x;
            y = config.smoothing * y + (1.0 - config.smoothing) * previous_y;
        }
        self.smoothed = Some((x, y));
        self.missed_frames = 0;

        let contrast = (diff_sum / area as f64 / 255.0).clamp(0.0, 1.0) as f32;
        let area_score = (area_fraction / (config.min_area_fraction * 4.0)).clamp(0.0, 1.0);
        let confidence = (0.55 * area_score + 0.45 * contrast).clamp(0.0, 1.0);

        Some(Detection {
            x,
            y,
            confidence,
            bbox: BoundingBox {
                x: min_x as u32,
                y: min_y as u32,
                width: (max_x - min_x + 1) as u32,
                height: (max_y - min_y + 1) as u32,
            },
        })
    }

    /// Record a frame without a usable foreground region.
    fn register_miss(&mut self) -> Option<Detection> {
        self.missed_frames = self.missed_frames.saturating_add(1);
        if self.missed_frames >= self.config.smoothing_reset_frames {
            self.smoothed = None;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{frame_with_rect, solid_frame};

    const WIDTH: u32 = 160;
    const HEIGHT: u32 = 120;
    const BACKGROUND: u8 = 20;
    const FOREGROUND: u8 = 220;

    fn seeded_detector() -> Detector {
        let mut detector = Detector::new();
        assert!(detector
            .detect(&solid_frame(WIDTH, HEIGHT, BACKGROUND))
            .is_none());
        detector
    }

    #[test]
    fn first_frame_initializes_background_without_detection() {
        let mut detector = Detector::new();
        assert!(detector
            .detect(&solid_frame(WIDTH, HEIGHT, BACKGROUND))
            .is_none());
        // Identical second frame: nothing changed, no detection.
        assert!(detector
            .detect(&solid_frame(WIDTH, HEIGHT, BACKGROUND))
            .is_none());
    }

    #[test]
    fn detects_bright_square_at_known_position() {
        let mut detector = seeded_detector();
        let rect = BoundingBox {
            x: 40,
            y: 20,
            width: 20,
            height: 20,
        };
        let detection = detector
            .detect(&frame_with_rect(
                WIDTH, HEIGHT, BACKGROUND, rect, FOREGROUND,
            ))
            .expect("a bright square must be detected");

        assert!((detection.x - 0.3125).abs() < 0.01, "x was {}", detection.x);
        assert!((detection.y - 0.25).abs() < 0.01, "y was {}", detection.y);
        assert_eq!(detection.bbox, rect);
        assert_eq!(detection.bbox.area(), 400);
        assert!(detection.confidence > 0.3);
        assert!(detection.confidence <= 1.0);
    }

    #[test]
    fn static_dark_scene_never_detects() {
        let mut detector = Detector::new();
        for _ in 0..10 {
            assert!(detector.detect(&solid_frame(WIDTH, HEIGHT, 0)).is_none());
        }
    }

    #[test]
    fn global_darkness_is_treated_as_illumination_change() {
        let mut detector = seeded_detector();
        // The whole scene goes dark at once: rebuild the background silently.
        assert!(detector.detect(&solid_frame(WIDTH, HEIGHT, 0)).is_none());
        assert!(detector.detect(&solid_frame(WIDTH, HEIGHT, 0)).is_none());
    }

    #[test]
    fn tiny_noise_is_rejected_by_minimum_area() {
        let mut detector = seeded_detector();
        let mut luma = vec![BACKGROUND; (WIDTH * HEIGHT) as usize];
        luma[(30 * WIDTH + 30) as usize] = 255;
        let frame = Frame::new(WIDTH, HEIGHT, luma).expect("valid frame");
        assert!(detector.detect(&frame).is_none());
    }

    #[test]
    fn smoothing_reduces_centroid_jitter() {
        let config = DetectorConfig {
            smoothing: 0.5,
            ..DetectorConfig::default()
        };
        let mut detector = Detector::with_config(config);
        assert!(detector
            .detect(&solid_frame(WIDTH, HEIGHT, BACKGROUND))
            .is_none());

        let first = detector
            .detect(&frame_with_rect(
                WIDTH,
                HEIGHT,
                BACKGROUND,
                BoundingBox {
                    x: 40,
                    y: 40,
                    width: 20,
                    height: 20,
                },
                FOREGROUND,
            ))
            .expect("first square");
        let second = detector
            .detect(&frame_with_rect(
                WIDTH,
                HEIGHT,
                BACKGROUND,
                BoundingBox {
                    x: 100,
                    y: 40,
                    width: 20,
                    height: 20,
                },
                FOREGROUND,
            ))
            .expect("second square");

        // Pixels 100..=119 have mean 109.5; +0.5 gives the pixel-center
        // coordinate 110, normalized to 110/160.
        let raw_second_x = (100.0 + 19.0 * 0.5 + 0.5) / WIDTH as f32;
        let expected = 0.5 * raw_second_x + 0.5 * first.x;
        assert!((second.x - expected).abs() < 1e-4, "x was {}", second.x);
        assert!(
            (second.x - first.x).abs() < (raw_second_x - first.x).abs(),
            "smoothed step must be smaller than the raw step"
        );
    }

    #[test]
    fn confidence_stays_within_bounds() {
        for foreground in [60u8, 90, 128, 200, 255] {
            let mut detector = seeded_detector();
            let rect = BoundingBox {
                x: 20,
                y: 20,
                width: 40,
                height: 40,
            };
            let detection = detector
                .detect(&frame_with_rect(
                    WIDTH, HEIGHT, BACKGROUND, rect, foreground,
                ))
                .expect("square must be detected");
            assert!(
                (0.0..=1.0).contains(&detection.confidence),
                "confidence was {}",
                detection.confidence
            );
            assert!((0.0..=1.0).contains(&detection.x));
            assert!((0.0..=1.0).contains(&detection.y));
        }
    }

    #[test]
    fn change_below_threshold_is_ignored() {
        let mut detector = seeded_detector();
        let rect = BoundingBox {
            x: 40,
            y: 40,
            width: 40,
            height: 40,
        };
        let subtle = frame_with_rect(WIDTH, HEIGHT, BACKGROUND, rect, BACKGROUND + 10);
        assert!(detector.detect(&subtle).is_none());
    }

    #[test]
    fn resolution_change_resets_background_model() {
        let mut detector = seeded_detector();
        assert!(detector.detect(&solid_frame(64, 48, BACKGROUND)).is_none());
        assert!(detector.detect(&solid_frame(64, 48, BACKGROUND)).is_none());
    }

    #[test]
    fn configuration_is_clamped_to_safe_ranges() {
        let detector = Detector::with_config(DetectorConfig {
            diff_threshold: 0,
            learn_rate: 9.0,
            min_area_fraction: 0.0,
            max_area_fraction: 0.0,
            smoothing: 0.0,
            smoothing_reset_frames: 0,
        });
        let config = detector.config();
        assert!(config.diff_threshold >= 1);
        assert!(config.learn_rate <= 0.5);
        assert!(config.min_area_fraction > 0.0);
        assert!(config.max_area_fraction > config.min_area_fraction);
        assert!(config.smoothing >= 0.01);
        assert!(config.smoothing <= 1.0);
        assert!(config.smoothing_reset_frames >= 1);
    }

    #[test]
    fn default_configuration_is_documented_values() {
        let config = DetectorConfig::default();
        assert_eq!(config.diff_threshold, 25);
        assert_eq!(config.learn_rate, 0.05);
        assert_eq!(config.min_area_fraction, 0.005);
        assert_eq!(config.max_area_fraction, 0.6);
        assert_eq!(config.smoothing, 0.35);
        assert_eq!(config.smoothing_reset_frames, 3);
    }
}
