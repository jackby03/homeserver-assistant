//! Bridge between the local vision service and the kiosk state machine.
//!
//! `hs-vision` connects to `/run/homecore/vision.sock` and writes one JSON
//! object per line. The daemon validates and maps each message into the kiosk
//! state, which broadcasts the corresponding `KioskEvent` to the browser.
//!
//! The wire format is intentionally trivial so the vision process and the
//! daemon can evolve independently:
//!
//! ```json
//! {"type":"tracking","point":{"x":0.42,"y":0.55,"confidence":0.87}}
//! {"type":"absence"}
//! {"type":"heartbeat","camera":"/dev/video0","fps":15}
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use hs_core::events::TrackingPoint;
use serde::Deserialize;

use crate::kiosk::KioskManager;

/// Maximum line size accepted from the vision socket (safety bound).
const MAX_LINE_BYTES: usize = 16 * 1024;

/// Message written by the vision service.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VisionMessage {
    /// A person was detected at a normalized position.
    Tracking {
        /// Gaze point in `[0, 1]` coordinates.
        point: TrackingPoint,
        /// Optional producer timestamp.
        #[serde(default)]
        timestamp: Option<DateTime<Utc>>,
    },
    /// Nobody is in front of the camera.
    Absence {
        /// Optional producer timestamp.
        #[serde(default)]
        timestamp: Option<DateTime<Utc>>,
    },
    /// Liveness signal from the camera pipeline.
    Heartbeat {
        /// Camera device used by the vision service.
        #[serde(default)]
        camera: Option<String>,
        /// Frames per second.
        #[serde(default)]
        fps: Option<u32>,
        /// Optional producer timestamp.
        #[serde(default)]
        timestamp: Option<DateTime<Utc>>,
    },
}

/// Parse one newline-delimited message from the vision socket.
#[must_use]
pub fn parse_vision_line(line: &str) -> Option<VisionMessage> {
    if line.len() > MAX_LINE_BYTES {
        return None;
    }
    serde_json::from_str(line.trim()).ok()
}

/// Apply a vision message to the kiosk state.
pub fn apply_vision_message(kiosk: &KioskManager, message: &VisionMessage) {
    match message {
        VisionMessage::Tracking { point, .. } => {
            if point.x.is_finite()
                && point.y.is_finite()
                && (0.0..=1.0).contains(&point.x)
                && (0.0..=1.0).contains(&point.y)
            {
                kiosk.set_tracking(Some(*point));
            } else {
                tracing::debug!(?point, "ignoring out-of-range tracking point");
            }
        }
        VisionMessage::Absence { .. } => kiosk.set_tracking(None),
        VisionMessage::Heartbeat { camera, fps, .. } => {
            tracing::trace!(?camera, ?fps, "vision heartbeat");
        }
    }
}

/// Run the vision Unix socket server until the process exits.
///
/// Only one vision service is expected; concurrent connections are accepted so
/// the service can restart without a stale socket blocking it.
///
/// # Errors
/// Returns an I/O error on non-Unix platforms or when the socket cannot be
/// created.
#[cfg(unix)]
pub async fn run_vision_bridge(
    socket_path: PathBuf,
    kiosk: Arc<KioskManager>,
) -> std::io::Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::UnixListener;

    if socket_path.exists() {
        // Remove a stale socket left by a previous daemon run.
        std::fs::remove_file(&socket_path)?;
    }
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o660));
    }
    tracing::info!(path = %socket_path.display(), "vision bridge listening");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let kiosk = Arc::clone(&kiosk);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        if let Some(message) = parse_vision_line(&line) {
                            apply_vision_message(&kiosk, &message);
                        } else {
                            tracing::debug!("discarding malformed vision message");
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "vision socket read error");
                        break;
                    }
                }
            }
            tracing::info!("vision client disconnected");
        });
    }
}

/// Non-Unix stub: the vision bridge is only supported on Linux.
///
/// # Errors
/// Always returns `Unsupported`.
#[cfg(not(unix))]
pub async fn run_vision_bridge(
    _socket_path: PathBuf,
    _kiosk: Arc<KioskManager>,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "vision bridge requires Unix sockets (Linux appliance)",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracking_message() {
        let line = r#"{"type":"tracking","point":{"x":0.25,"y":0.75,"confidence":0.9}}"#;
        let message = parse_vision_line(line).unwrap();
        assert!(matches!(
            message,
            VisionMessage::Tracking { point, .. }
                if (point.x - 0.25).abs() < f32::EPSILON
                    && (point.y - 0.75).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn parses_absence_and_heartbeat() {
        assert_eq!(
            parse_vision_line(r#"{"type":"absence"}"#).unwrap(),
            VisionMessage::Absence { timestamp: None }
        );
        let heartbeat =
            parse_vision_line(r#"{"type":"heartbeat","camera":"/dev/video0","fps":15}"#).unwrap();
        assert!(matches!(
            heartbeat,
            VisionMessage::Heartbeat { fps: Some(15), .. }
        ));
    }

    #[test]
    fn rejects_garbage_and_oversized_lines() {
        assert!(parse_vision_line("not json").is_none());
        assert!(parse_vision_line(r#"{"type":"unknown"}"#).is_none());
        let oversized = "x".repeat(MAX_LINE_BYTES + 1);
        assert!(parse_vision_line(&oversized).is_none());
    }
}
