//! The tracking service: pacing, presence state machine and event publishing.
//!
//! [`VisionService`] owns one [`FrameSource`], one [`Detector`] and one
//! [`EventSink`]. [`VisionService::run`] ticks at the configured frame rate,
//! turns detections into [`VisionEvent`]s and stops gracefully when the
//! shutdown watch is set. [`VisionService::run_once`] performs a single frame
//! step, which keeps tests and the `--once` CLI mode deterministic.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::detector::{Detector, DetectorConfig};
use crate::event::{TrackingPoint, VisionEvent};
use crate::frame::{Frame, FrameSource};
use crate::sink::EventSink;
use crate::{VisionError, VisionResult};

/// Interval between liveness heartbeats published on the socket.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// Tracking events are refreshed at roughly this rate while a person is present.
const TRACKING_REFRESH_HZ: u32 = 10;
/// Consecutive frame-source errors tolerated before the service aborts.
const MAX_CONSECUTIVE_FRAME_ERRORS: u32 = 50;

/// Runtime configuration for [`VisionService`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisionConfig {
    /// Capture frame rate in frames per second, `1..=120`.
    pub fps: u32,
    /// Centroid smoothing alpha: `1.0` disables smoothing, small values are
    /// heavier but smoother.
    pub smoothing: f32,
    /// Minimum detector confidence in `[0, 1]` required to emit tracking.
    pub min_confidence: f32,
    /// Consecutive frames without a detection before absence is emitted.
    pub absent_after_frames: u32,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            fps: 15,
            smoothing: 0.35,
            min_confidence: 0.35,
            absent_after_frames: 15,
        }
    }
}

impl VisionConfig {
    /// Lowest accepted frame rate.
    pub const MIN_FPS: u32 = 1;
    /// Highest accepted frame rate.
    pub const MAX_FPS: u32 = 120;

    /// Validate the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::Config`] when a field is outside its documented
    /// range.
    pub fn validate(&self) -> VisionResult<()> {
        if !(Self::MIN_FPS..=Self::MAX_FPS).contains(&self.fps) {
            return Err(VisionError::Config(format!(
                "fps must be within {}..={}, got {}",
                Self::MIN_FPS,
                Self::MAX_FPS,
                self.fps
            )));
        }
        if !(0.0..=1.0).contains(&self.smoothing) || self.smoothing <= 0.0 {
            return Err(VisionError::Config(format!(
                "smoothing must be within 0.0..=1.0 exclusive of 0.0, got {}",
                self.smoothing
            )));
        }
        if !(0.0..=1.0).contains(&self.min_confidence) {
            return Err(VisionError::Config(format!(
                "min_confidence must be within 0.0..=1.0, got {}",
                self.min_confidence
            )));
        }
        if self.absent_after_frames == 0 {
            return Err(VisionError::Config(
                "absent_after_frames must be at least 1".to_string(),
            ));
        }
        Ok(())
    }

    /// Duration between two frame ticks.
    #[must_use]
    pub fn frame_interval(&self) -> Duration {
        Duration::from_secs_f64(1.0 / f64::from(self.fps.max(1)))
    }
}

/// Presence state machine used to debounce absence reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    /// Nothing decided yet (service just started).
    Unknown,
    /// A person is currently tracked.
    Present,
    /// Absence has been reported; stay quiet until a person appears again.
    Absent,
}

/// Camera tracking service.
///
/// It reads frames from a [`FrameSource`], runs the pure-Rust [`Detector`] and
/// publishes [`VisionEvent`]s to an [`EventSink`]. Presence transitions are
/// emitted immediately; steady-state tracking events are refreshed at roughly
/// `fps / 10` frames; absence is reported once per transition after
/// [`VisionConfig::absent_after_frames`] consecutive misses.
pub struct VisionService {
    source: Box<dyn FrameSource>,
    detector: Detector,
    sink: Arc<dyn EventSink>,
    config: VisionConfig,
    camera: String,
    frame_interval: Duration,
    tracking_refresh_frames: u32,
    presence: Presence,
    absent_frames: u32,
    frames_since_tracking: u32,
    consecutive_frame_errors: u32,
}

impl VisionService {
    /// Build a service from its ports and configuration.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::Config`] when `config` is invalid.
    pub fn new(
        source: Box<dyn FrameSource>,
        sink: Arc<dyn EventSink>,
        config: VisionConfig,
    ) -> VisionResult<Self> {
        config.validate()?;
        let camera = source.description();
        let detector = Detector::with_config(DetectorConfig {
            smoothing: config.smoothing,
            ..DetectorConfig::default()
        });
        Ok(Self {
            source,
            detector,
            sink,
            config,
            camera,
            frame_interval: config.frame_interval(),
            tracking_refresh_frames: (config.fps / TRACKING_REFRESH_HZ).max(1),
            presence: Presence::Unknown,
            absent_frames: 0,
            frames_since_tracking: 0,
            consecutive_frame_errors: 0,
        })
    }

    /// Effective configuration.
    #[must_use]
    pub fn config(&self) -> &VisionConfig {
        &self.config
    }

    /// Camera description used in logs and heartbeat events.
    #[must_use]
    pub fn camera(&self) -> &str {
        &self.camera
    }

    /// Process exactly one frame and return the event that was published.
    ///
    /// # Errors
    ///
    /// Propagates frame-source failures and sink failures.
    pub async fn run_once(&mut self) -> VisionResult<Option<VisionEvent>> {
        let frame = self.source.next_frame().await?;
        self.process_frame(&frame).await
    }

    /// Run detection and publishing for an already acquired frame.
    ///
    /// Returns the published event, if any. This is the unit the service loop
    /// and tests share.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::InvalidEvent`] if the detector produced a value
    /// outside the normalized range and propagates sink failures.
    pub async fn process_frame(&mut self, frame: &Frame) -> VisionResult<Option<VisionEvent>> {
        let detection = self
            .detector
            .detect(frame)
            .filter(|detection| detection.confidence >= self.config.min_confidence);

        if let Some(detection) = detection {
            self.absent_frames = 0;
            let was_present = matches!(self.presence, Presence::Present);
            self.presence = Presence::Present;
            let should_publish =
                !was_present || self.frames_since_tracking >= self.tracking_refresh_frames;
            if !should_publish {
                self.frames_since_tracking = self.frames_since_tracking.saturating_add(1);
                return Ok(None);
            }
            let point = TrackingPoint::new(detection.x, detection.y, detection.confidence)?;
            let event = VisionEvent::tracking(point);
            self.sink.publish(&event).await?;
            self.frames_since_tracking = 0;
            return Ok(Some(event));
        }

        self.frames_since_tracking = 0;
        self.absent_frames = self.absent_frames.saturating_add(1);
        if !matches!(self.presence, Presence::Absent)
            && self.absent_frames >= self.config.absent_after_frames
        {
            self.presence = Presence::Absent;
            let event = VisionEvent::absence();
            self.sink.publish(&event).await?;
            return Ok(Some(event));
        }
        Ok(None)
    }

    /// Publish a heartbeat event immediately.
    ///
    /// # Errors
    ///
    /// Propagates sink failures.
    pub async fn publish_heartbeat(&mut self) -> VisionResult<()> {
        let event = VisionEvent::heartbeat(self.camera.clone(), self.config.fps);
        self.sink.publish(&event).await
    }

    /// Run the tracking loop until shutdown is requested or the source ends.
    ///
    /// The service ticks at [`VisionConfig::fps`], emits presence transitions
    /// immediately, refreshes tracking (and publishes heartbeats) while running,
    /// and shuts down gracefully when `shutdown` becomes `true` or its sender is
    /// dropped.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::Config`] never at this point (validated in
    /// [`VisionService::new`]) and propagates fatal frame-source failures after
    /// [`MAX_CONSECUTIVE_FRAME_ERRORS`] consecutive errors.
    pub async fn run(&mut self, mut shutdown: watch::Receiver<bool>) -> VisionResult<()> {
        if *shutdown.borrow() {
            tracing::info!(camera = %self.camera, "vision service stopped before starting");
            return Ok(());
        }

        let mut ticker = tokio::time::interval(self.frame_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_heartbeat = tokio::time::Instant::now();
        tracing::info!(camera = %self.camera, fps = self.config.fps, "vision service started");

        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                _ = ticker.tick() => {
                    match self.step(&mut last_heartbeat).await {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(error) => {
                            tracing::error!(%error, "vision service stopped");
                            return Err(error);
                        }
                    }
                }
            }
        }

        tracing::info!(camera = %self.camera, "vision service stopped");
        Ok(())
    }

    /// Acquire, detect and publish for a single tick.
    ///
    /// Returns `Ok(false)` when the frame source is exhausted (a clean stop) and
    /// `Ok(true)` when the loop should continue.
    async fn step(&mut self, last_heartbeat: &mut tokio::time::Instant) -> VisionResult<bool> {
        match self.source.next_frame().await {
            Ok(frame) => {
                self.consecutive_frame_errors = 0;
                match self.process_frame(&frame).await {
                    Ok(Some(event)) => {
                        tracing::debug!(kind = event.kind(), "vision event published");
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "failed to publish vision event, continuing");
                    }
                }
            }
            Err(VisionError::SourceExhausted) => {
                tracing::info!(camera = %self.camera, "frame source exhausted");
                return Ok(false);
            }
            Err(error) => {
                self.consecutive_frame_errors = self.consecutive_frame_errors.saturating_add(1);
                tracing::warn!(
                    %error,
                    consecutive = self.consecutive_frame_errors,
                    "failed to read frame"
                );
                if self.consecutive_frame_errors >= MAX_CONSECUTIVE_FRAME_ERRORS {
                    return Err(error);
                }
            }
        }

        if last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL {
            match self.publish_heartbeat().await {
                Ok(()) => tracing::debug!(camera = %self.camera, "vision heartbeat published"),
                Err(error) => {
                    tracing::warn!(%error, "failed to publish vision heartbeat, continuing");
                }
            }
            *last_heartbeat = tokio::time::Instant::now();
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::ChannelSink;
    use crate::synthetic::SyntheticSource;
    use crate::test_support::{frame_with_rect, solid_frame, ScriptedSource};
    use crate::BoundingBox;

    const WIDTH: u32 = 160;
    const HEIGHT: u32 = 120;
    const BACKGROUND: u8 = 20;
    const FOREGROUND: u8 = 220;

    fn test_config() -> VisionConfig {
        VisionConfig {
            fps: 15,
            smoothing: 1.0,
            min_confidence: 0.2,
            absent_after_frames: 2,
        }
    }

    fn square(x: u32, y: u32) -> BoundingBox {
        BoundingBox {
            x,
            y,
            width: 20,
            height: 20,
        }
    }

    fn build_service(
        frames: Vec<Frame>,
    ) -> (VisionService, tokio::sync::mpsc::Receiver<VisionEvent>) {
        let (sink, receiver) = ChannelSink::new(16);
        let service = VisionService::new(
            Box::new(ScriptedSource::new(frames)),
            Arc::new(sink),
            test_config(),
        )
        .expect("valid vision configuration");
        (service, receiver)
    }

    #[tokio::test]
    async fn emits_tracking_after_background_initialization() {
        let frames = vec![
            solid_frame(WIDTH, HEIGHT, BACKGROUND),
            frame_with_rect(WIDTH, HEIGHT, BACKGROUND, square(40, 20), FOREGROUND),
        ];
        let (mut service, mut receiver) = build_service(frames);

        assert!(service.run_once().await.expect("first frame").is_none());
        let event = service
            .run_once()
            .await
            .expect("second frame")
            .expect("tracking event");
        assert_eq!(event.kind(), "tracking");
        if let VisionEvent::Tracking { point, .. } = &event {
            assert!((point.x - 0.3125).abs() < 0.01, "x was {}", point.x);
            assert!((point.y - 0.25).abs() < 0.01, "y was {}", point.y);
            assert!(point.confidence >= test_config().min_confidence);
            assert!((0.0..=1.0).contains(&point.x));
            assert!((0.0..=1.0).contains(&point.y));
        }

        let received = receiver.try_recv().expect("event delivered to sink");
        assert_eq!(received, event);
    }

    #[tokio::test]
    async fn emits_absence_after_consecutive_misses_and_not_repeatedly() {
        let frames = vec![
            solid_frame(WIDTH, HEIGHT, BACKGROUND),
            frame_with_rect(WIDTH, HEIGHT, BACKGROUND, square(40, 20), FOREGROUND),
            solid_frame(WIDTH, HEIGHT, BACKGROUND),
            solid_frame(WIDTH, HEIGHT, BACKGROUND),
            solid_frame(WIDTH, HEIGHT, BACKGROUND),
        ];
        let (mut service, _receiver) = build_service(frames);

        assert!(service
            .run_once()
            .await
            .expect("background frame")
            .is_none());
        let tracking = service
            .run_once()
            .await
            .expect("tracking frame")
            .expect("tracking event");
        assert_eq!(tracking.kind(), "tracking");

        assert!(service.run_once().await.expect("first miss").is_none());
        let absence = service
            .run_once()
            .await
            .expect("second miss")
            .expect("absence event");
        assert_eq!(absence.kind(), "absence");
        assert!(matches!(absence, VisionEvent::Absence { .. }));

        // Already absent: no duplicate absence event.
        assert!(service.run_once().await.expect("steady absence").is_none());
    }

    #[tokio::test]
    async fn reports_absence_without_any_prior_detection() {
        let frames = vec![
            solid_frame(WIDTH, HEIGHT, BACKGROUND),
            solid_frame(WIDTH, HEIGHT, BACKGROUND),
        ];
        let (mut service, _receiver) = build_service(frames);
        assert!(service.run_once().await.expect("initial frame").is_none());
        let event = service
            .run_once()
            .await
            .expect("second frame")
            .expect("absence event");
        assert_eq!(event.kind(), "absence");
    }

    #[tokio::test]
    async fn source_exhaustion_surfaces_as_an_error_from_run_once() {
        let (mut service, _receiver) = build_service(Vec::new());
        assert!(matches!(
            service.run_once().await,
            Err(VisionError::SourceExhausted)
        ));
    }

    #[tokio::test]
    async fn heartbeat_carries_camera_and_frame_rate() {
        let (mut service, mut receiver) = build_service(Vec::new());
        service.publish_heartbeat().await.expect("heartbeat");
        let event = receiver.try_recv().expect("heartbeat delivered");
        match event {
            VisionEvent::Heartbeat { camera, fps, .. } => {
                assert_eq!(camera, "scripted test camera");
                assert_eq!(fps, test_config().fps);
            }
            other => panic!("expected heartbeat, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_configurations_are_rejected() {
        fn build(config: VisionConfig) -> VisionResult<VisionService> {
            let (sink, receiver) = ChannelSink::new(1);
            drop(receiver);
            VisionService::new(Box::new(SyntheticSource::default()), Arc::new(sink), config)
        }

        assert!(build(VisionConfig {
            fps: 0,
            ..test_config()
        })
        .is_err());
        assert!(build(VisionConfig {
            smoothing: 0.0,
            ..test_config()
        })
        .is_err());
        assert!(build(VisionConfig {
            min_confidence: 1.5,
            ..test_config()
        })
        .is_err());
        assert!(build(VisionConfig {
            absent_after_frames: 0,
            ..test_config()
        })
        .is_err());
    }

    #[tokio::test]
    async fn run_returns_immediately_when_shutdown_is_already_requested() {
        let (sink, _receiver) = ChannelSink::new(4);
        let mut service = VisionService::new(
            Box::new(SyntheticSource::default()),
            Arc::new(sink),
            test_config(),
        )
        .expect("valid configuration");
        let (sender, receiver) = watch::channel(true);
        service.run(receiver).await.expect("clean shutdown");
        drop(sender);
    }

    #[tokio::test]
    async fn run_stops_cleanly_when_the_source_is_exhausted() {
        let (sink, _receiver) = ChannelSink::new(4);
        let mut service = VisionService::new(
            Box::new(ScriptedSource::new(vec![
                solid_frame(WIDTH, HEIGHT, BACKGROUND),
                solid_frame(WIDTH, HEIGHT, BACKGROUND),
            ])),
            Arc::new(sink),
            VisionConfig {
                fps: 120,
                ..test_config()
            },
        )
        .expect("valid configuration");
        let (_sender, receiver) = watch::channel(false);
        service
            .run(receiver)
            .await
            .expect("exhausted source is a clean stop");
    }
}
