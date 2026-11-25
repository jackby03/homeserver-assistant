//! Newline-delimited JSON wire format consumed by `hs-daemon`.
//!
//! The vision service writes exactly one JSON object per line over a local Unix
//! domain socket. The daemon reads lines, converts them into
//! [`hs_core::events::KioskEvent::Tracking`] and pushes them to the kiosk
//! WebSocket.
//!
//! This module intentionally defines its own [`TrackingPoint`] instead of
//! depending on the kiosk event model, so the wire contract is owned by the
//! vision service and can evolve independently. The mapping is mechanical:
//!
//! | `hs-vision` wire event | daemon action |
//! |---|---|
//! | `{"type":"tracking","point":{...},"timestamp":"..."}` | `KioskEvent::Tracking { point: Some(point.into()) }` |
//! | `{"type":"absence","timestamp":"..."}` | `KioskEvent::Tracking { point: None }` |
//! | `{"type":"heartbeat",...}` | liveness bookkeeping only, not forwarded |
//!
//! Coordinates are normalized: `x` grows left to right, `y` grows top to
//! bottom, both in `[0, 1]`; `confidence` is in `[0, 1]`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{VisionError, VisionResult};

/// Normalized gaze point published on the vision socket.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrackingPoint {
    /// Horizontal coordinate, `0.0` left to `1.0` right.
    pub x: f32,
    /// Vertical coordinate, `0.0` top to `1.0` bottom.
    pub y: f32,
    /// Detector confidence in `[0, 1]`.
    pub confidence: f32,
}

impl TrackingPoint {
    /// Validate and build a normalized gaze point.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::InvalidEvent`] when a coordinate is not finite or
    /// outside `[0, 1]`.
    pub fn new(x: f32, y: f32, confidence: f32) -> VisionResult<Self> {
        let in_range = |value: f32| (0.0..=1.0).contains(&value);
        if !in_range(x) || !in_range(y) || !in_range(confidence) {
            return Err(VisionError::InvalidEvent(format!(
                "normalized values must be finite and within 0..=1 (x={x}, y={y}, confidence={confidence})"
            )));
        }
        Ok(Self { x, y, confidence })
    }
}

/// Conversion into the kiosk event model, applied by `hs-daemon`.
impl From<TrackingPoint> for hs_core::events::TrackingPoint {
    fn from(point: TrackingPoint) -> Self {
        Self {
            x: point.x,
            y: point.y,
            confidence: point.confidence,
        }
    }
}

/// One event published by the vision service.
///
/// Serialized as internally tagged JSON using the `type` discriminator, e.g.
/// `{"type":"tracking","point":{"x":0.42,"y":0.55,"confidence":0.87},"timestamp":"2026-01-02T03:04:05Z"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VisionEvent {
    /// A person is present at the gaze point.
    Tracking {
        /// Smoothed, normalized gaze point.
        point: TrackingPoint,
        /// UTC instant at which the frame was processed.
        timestamp: DateTime<Utc>,
    },
    /// Nobody has been tracked for `absent_after_frames` consecutive frames.
    Absence {
        /// UTC instant at which absence was declared.
        timestamp: DateTime<Utc>,
    },
    /// Periodic liveness message with capture diagnostics.
    Heartbeat {
        /// Camera description; the device node for real cameras.
        camera: String,
        /// Configured capture frame rate.
        fps: u32,
        /// UTC instant at which the heartbeat was emitted.
        timestamp: DateTime<Utc>,
    },
}

impl VisionEvent {
    /// Build a [`VisionEvent::Tracking`] stamped with the current UTC time.
    #[must_use]
    pub fn tracking(point: TrackingPoint) -> Self {
        Self::Tracking {
            point,
            timestamp: Utc::now(),
        }
    }

    /// Build a [`VisionEvent::Absence`] stamped with the current UTC time.
    #[must_use]
    pub fn absence() -> Self {
        Self::Absence {
            timestamp: Utc::now(),
        }
    }

    /// Build a [`VisionEvent::Heartbeat`] stamped with the current UTC time.
    #[must_use]
    pub fn heartbeat(camera: impl Into<String>, fps: u32) -> Self {
        Self::Heartbeat {
            camera: camera.into(),
            fps,
            timestamp: Utc::now(),
        }
    }

    /// Stable snake_case discriminator (`"tracking"`, `"absence"`, `"heartbeat"`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Tracking { .. } => "tracking",
            Self::Absence { .. } => "absence",
            Self::Heartbeat { .. } => "heartbeat",
        }
    }

    /// UTC timestamp carried by the event.
    #[must_use]
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::Tracking { timestamp, .. }
            | Self::Absence { timestamp }
            | Self::Heartbeat { timestamp, .. } => *timestamp,
        }
    }

    /// Serialize the event as one JSON line, including the trailing newline.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::Serialization`] when the event cannot be encoded.
    pub fn to_json_line(&self) -> VisionResult<String> {
        let mut line = serde_json::to_string(self)?;
        line.push('\n');
        Ok(line)
    }

    /// Parse one JSON line emitted by the vision service.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::InvalidEvent`] for an empty line and
    /// [`VisionError::Serialization`] for malformed JSON or an unknown event.
    pub fn from_json_line(line: &str) -> VisionResult<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err(VisionError::InvalidEvent(
                "empty vision event line".to_string(),
            ));
        }
        serde_json::from_str(trimmed).map_err(VisionError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_timestamp() -> DateTime<Utc> {
        match Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5) {
            chrono::LocalResult::Single(value) => value,
            other => panic!("invalid test timestamp: {other:?}"),
        }
    }

    fn fixed_point() -> TrackingPoint {
        TrackingPoint::new(0.42, 0.55, 0.87).expect("valid point")
    }

    #[test]
    fn tracking_event_matches_wire_contract() {
        let event = VisionEvent::Tracking {
            point: fixed_point(),
            timestamp: fixed_timestamp(),
        };
        let line = event.to_json_line().expect("serialize tracking event");
        assert_eq!(
            line,
            "{\"type\":\"tracking\",\"point\":{\"x\":0.42,\"y\":0.55,\"confidence\":0.87},\"timestamp\":\"2026-01-02T03:04:05Z\"}\n"
        );
    }

    #[test]
    fn absence_event_matches_wire_contract() {
        let event = VisionEvent::Absence {
            timestamp: fixed_timestamp(),
        };
        let line = event.to_json_line().expect("serialize absence event");
        assert_eq!(
            line,
            "{\"type\":\"absence\",\"timestamp\":\"2026-01-02T03:04:05Z\"}\n"
        );
    }

    #[test]
    fn heartbeat_event_matches_wire_contract() {
        let event = VisionEvent::Heartbeat {
            camera: "/dev/video0".to_string(),
            fps: 15,
            timestamp: fixed_timestamp(),
        };
        let line = event.to_json_line().expect("serialize heartbeat event");
        assert_eq!(
            line,
            "{\"type\":\"heartbeat\",\"camera\":\"/dev/video0\",\"fps\":15,\"timestamp\":\"2026-01-02T03:04:05Z\"}\n"
        );
    }

    #[test]
    fn json_keys_and_types_are_stable() {
        let event = VisionEvent::Tracking {
            point: fixed_point(),
            timestamp: fixed_timestamp(),
        };
        let json = serde_json::to_value(&event).expect("serialize to value");
        assert_eq!(json["type"], "tracking");
        assert!(json["point"]["x"].is_number());
        assert!(json["point"]["y"].is_number());
        assert!(json["point"]["confidence"].is_number());
        assert!(json["timestamp"].is_string());

        let absence = VisionEvent::Absence {
            timestamp: fixed_timestamp(),
        };
        let json = serde_json::to_value(&absence).expect("serialize to value");
        assert_eq!(json["type"], "absence");
        assert_eq!(json.as_object().map(serde_json::Map::len), Some(2));
    }

    #[test]
    fn events_round_trip_through_wire_lines() {
        let events = [
            VisionEvent::Tracking {
                point: fixed_point(),
                timestamp: fixed_timestamp(),
            },
            VisionEvent::Absence {
                timestamp: fixed_timestamp(),
            },
            VisionEvent::Heartbeat {
                camera: "synthetic camera 160x120 @ 15 fps".to_string(),
                fps: 15,
                timestamp: fixed_timestamp(),
            },
        ];
        for event in events {
            let line = event.to_json_line().expect("serialize");
            let parsed = VisionEvent::from_json_line(&line).expect("parse");
            assert_eq!(parsed, event);
            assert_eq!(parsed.kind(), event.kind());
            assert_eq!(parsed.timestamp(), event.timestamp());
        }
    }

    #[test]
    fn empty_lines_are_rejected() {
        assert!(matches!(
            VisionEvent::from_json_line("  \n"),
            Err(VisionError::InvalidEvent(_))
        ));
        assert!(VisionEvent::from_json_line("not json").is_err());
        assert!(VisionEvent::from_json_line("{\"type\":\"unknown\"}").is_err());
    }

    #[test]
    fn tracking_point_validation_rejects_invalid_values() {
        assert!(TrackingPoint::new(0.0, 0.0, 0.0).is_ok());
        assert!(TrackingPoint::new(1.0, 1.0, 1.0).is_ok());
        assert!(TrackingPoint::new(1.01, 0.5, 0.5).is_err());
        assert!(TrackingPoint::new(0.5, -0.01, 0.5).is_err());
        assert!(TrackingPoint::new(0.5, 0.5, f32::NAN).is_err());
        assert!(TrackingPoint::new(f32::INFINITY, 0.5, 0.5).is_err());
    }

    #[test]
    fn tracking_point_maps_to_the_kiosk_event_model() {
        let conversion: hs_core::events::TrackingPoint = fixed_point().into();
        assert_eq!(conversion.x, 0.42);
        assert_eq!(conversion.y, 0.55);
        assert_eq!(conversion.confidence, 0.87);
    }
}
