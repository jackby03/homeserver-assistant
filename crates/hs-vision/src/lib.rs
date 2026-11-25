//! Local-first camera tracking service and normalized (x, y) event bridge.
//!
//! `hs-vision` turns a local camera into a small stream of normalized gaze
//! points that the `hs-daemon` forwards to the wall kiosk. Everything runs on
//! the appliance: frames are processed in memory, are never written to disk or
//! uploaded, and only normalized coordinates (or, for the optional enrollment
//! helper, face embeddings produced locally by Python) leave the process.
//!
//! # Pipeline
//!
//! 1. A [`FrameSource`] produces grayscale frames: [`V4l2Source`] on Linux or
//!    the deterministic [`SyntheticSource`] for demos and tests.
//! 2. [`Detector`] maintains a running background model, extracts the
//!    foreground blob and returns a smoothed, normalized [`TrackingPoint`].
//! 3. [`VisionService`] debounces presence/absence, adds timestamps and
//!    publishes [`VisionEvent`]s at the configured frame rate.
//! 4. An [`EventSink`] delivers events: [`UnixSocketSink`] writes
//!    newline-delimited JSON to `/run/homecore/vision.sock`, [`ChannelSink`]
//!    feeds the daemon in-process, and [`NullSink`] discards them.
//!
//! # Wire format
//!
//! One internally tagged JSON object per line:
//!
//! ```json
//! {"type":"tracking","point":{"x":0.42,"y":0.55,"confidence":0.87},"timestamp":"2026-01-02T03:04:05Z"}
//! {"type":"absence","timestamp":"2026-01-02T03:04:06Z"}
//! {"type":"heartbeat","camera":"/dev/video0","fps":15,"timestamp":"2026-01-02T03:04:15Z"}
//! ```
//!
//! The daemon maps a tracking event to
//! [`hs_core::events::KioskEvent::Tracking`] with the point converted through
//! the [`From`] implementation in [`event`], and an absence event to the same
//! kiosk event with `point: None`.
//!
//! # Binary
//!
//! The `hs-vision` binary (see `src/main.rs`) runs the service:
//!
//! ```text
//! hs-vision --device /dev/video0 --socket /run/homecore/vision.sock --fps 15
//! hs-vision --synthetic --stdout --once
//! ```
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//!
//! use hs_vision::{
//!     ChannelSink, Detection, Detector, Frame, FrameSource, SyntheticSource,
//!     VisionConfig, VisionService,
//! };
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut source = SyntheticSource::default();
//! let frame: Frame = source.next_frame().await?;
//! assert_eq!(frame.dimensions(), (160, 120));
//!
//! let mut detector = Detector::new();
//! let _first: Option<Detection> = detector.detect(&frame); // seeds background
//!
//! let (sink, _events) = ChannelSink::new(8);
//! let service = VisionService::new(
//!     Box::new(SyntheticSource::default()),
//!     Arc::new(sink),
//!     VisionConfig::default(),
//! )?;
//! assert_eq!(service.camera(), "synthetic camera 160x120 @ 15 fps");
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]

mod convert;
pub mod detector;
pub mod error;
pub mod event;
pub mod frame;
pub mod service;
pub mod sink;
pub mod synthetic;
pub mod v4l2;

#[cfg(test)]
mod test_support;

pub use detector::{BoundingBox, Detection, Detector, DetectorConfig};
pub use error::{VisionError, VisionResult};
pub use event::{TrackingPoint, VisionEvent};
pub use frame::{Frame, FrameSource};
pub use service::{VisionConfig, VisionService};
pub use sink::{ChannelSink, EventSink, NullSink, UnixSocketSink};
pub use synthetic::SyntheticSource;
pub use v4l2::V4l2Source;
