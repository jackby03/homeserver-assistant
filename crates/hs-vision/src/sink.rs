//! Event sinks: where the service delivers processed tracking events.
//!
//! The production sink writes newline-delimited JSON to the daemon's Unix
//! domain socket. [`ChannelSink`] is used when the daemon embeds the service
//! in-process or in tests, and [`NullSink`] discards events for dry runs.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::{VisionError, VisionEvent, VisionResult};

/// Destination for [`VisionEvent`]s produced by [`crate::VisionService`].
///
/// Sinks must be cheap to share behind an `Arc`; publishing takes `&self` and
/// implementations own any interior mutability they need.
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Publish a single event.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::Sink`] or [`VisionError::Unavailable`] when the
    /// event cannot be delivered. The service treats sink failures as
    /// non-fatal and retries with the next event.
    async fn publish(&self, event: &VisionEvent) -> VisionResult<()>;
}

/// Sink that discards every event.
///
/// Useful for dry runs, benchmarks and tests that only care about detection.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullSink;

#[async_trait]
impl EventSink for NullSink {
    async fn publish(&self, _event: &VisionEvent) -> VisionResult<()> {
        Ok(())
    }
}

/// Sink that forwards events into a bounded Tokio channel.
///
/// [`ChannelSink::new`] returns the sink and the receiver side. The daemon uses
/// it to run the vision service in-process and bridge events to its WebSocket
/// hub; tests use it to assert on the emitted event stream.
#[derive(Debug)]
pub struct ChannelSink {
    sender: mpsc::Sender<VisionEvent>,
}

impl ChannelSink {
    /// Create a channel sink and its receiver.
    ///
    /// `capacity` values below 1 are clamped to 1: Tokio channels must be
    /// bounded by a positive number.
    #[must_use]
    pub fn new(capacity: usize) -> (Self, mpsc::Receiver<VisionEvent>) {
        let (sender, receiver) = mpsc::channel(capacity.max(1));
        (Self { sender }, receiver)
    }
}

#[async_trait]
impl EventSink for ChannelSink {
    async fn publish(&self, event: &VisionEvent) -> VisionResult<()> {
        self.sender
            .send(event.clone())
            .await
            .map_err(|_| VisionError::Sink("vision event channel receiver was dropped".to_string()))
    }
}

/// Sink that writes newline-delimited JSON to a local Unix domain socket.
///
/// The connection is opened lazily on the first [`EventSink::publish`] call and
/// re-established automatically after a failed write, so the vision service may
/// start before the daemon has created its listening socket. Events stay on the
/// local machine: the socket path is a filesystem path, not a network address.
#[derive(Debug)]
pub struct UnixSocketSink {
    path: PathBuf,
    #[cfg(unix)]
    connection: tokio::sync::Mutex<Option<tokio::net::UnixStream>>,
}

impl UnixSocketSink {
    /// Create a sink that publishes to the Unix socket at `path`.
    ///
    /// No connection is attempted here; the first publish connects. This lets
    /// the service boot independently from the daemon.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::Config`] for an empty path and
    /// [`VisionError::Unavailable`] when Unix domain sockets are unsupported on
    /// the current platform.
    pub fn connect(path: impl Into<PathBuf>) -> VisionResult<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(VisionError::Config(
                "vision socket path must not be empty".to_string(),
            ));
        }
        Self::connect_platform(path)
    }

    /// Socket path this sink publishes to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(unix)]
    fn connect_platform(path: PathBuf) -> VisionResult<Self> {
        Ok(Self {
            path,
            connection: tokio::sync::Mutex::new(None),
        })
    }

    #[cfg(not(unix))]
    fn connect_platform(path: PathBuf) -> VisionResult<Self> {
        Err(VisionError::Unavailable(format!(
            "Unix domain sockets are not supported on this platform: {}",
            path.display()
        )))
    }

    #[cfg(unix)]
    async fn publish_unix(&self, event: &VisionEvent) -> VisionResult<()> {
        use tokio::io::AsyncWriteExt;

        let line = event.to_json_line()?;
        let mut connection = self.connection.lock().await;
        for attempt in 0..2 {
            if connection.is_none() {
                let stream =
                    tokio::net::UnixStream::connect(&self.path)
                        .await
                        .map_err(|error| {
                            VisionError::Unavailable(format!(
                                "cannot connect to vision socket {}: {error}",
                                self.path.display()
                            ))
                        })?;
                *connection = Some(stream);
            }
            let write_result = if let Some(stream) = connection.as_mut() {
                match stream.write_all(line.as_bytes()).await {
                    Ok(()) => stream.flush().await,
                    Err(error) => Err(error),
                }
            } else {
                Err(std::io::Error::other(
                    "vision socket connection disappeared unexpectedly",
                ))
            };
            match write_result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    *connection = None;
                    if attempt == 1 {
                        return Err(VisionError::Io(error));
                    }
                    tracing::warn!(
                        socket = %self.path.display(),
                        %error,
                        "vision socket write failed, reconnecting"
                    );
                }
            }
        }
        Err(VisionError::Sink(
            "vision socket remained unavailable after reconnect".to_string(),
        ))
    }
}

#[async_trait]
impl EventSink for UnixSocketSink {
    async fn publish(&self, event: &VisionEvent) -> VisionResult<()> {
        #[cfg(unix)]
        {
            self.publish_unix(event).await
        }
        #[cfg(not(unix))]
        {
            let _ = event;
            Err(VisionError::Unavailable(
                "Unix domain sockets are not supported on this platform".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackingPoint;

    fn sample_event() -> VisionEvent {
        VisionEvent::tracking(TrackingPoint::new(0.25, 0.75, 0.9).expect("valid point"))
    }

    #[tokio::test]
    async fn null_sink_accepts_events() {
        NullSink.publish(&sample_event()).await.expect("null sink");
    }

    #[tokio::test]
    async fn channel_sink_delivers_events() {
        let (sink, mut receiver) = ChannelSink::new(4);
        let event = sample_event();
        sink.publish(&event).await.expect("channel publish");
        let received = receiver.recv().await.expect("event received");
        assert_eq!(received, event);
    }

    #[tokio::test]
    async fn channel_sink_clamps_zero_capacity() {
        let (sink, mut receiver) = ChannelSink::new(0);
        sink.publish(&sample_event())
            .await
            .expect("channel publish");
        assert!(receiver.recv().await.is_some());
    }

    #[tokio::test]
    async fn channel_sink_reports_dropped_receiver() {
        let (sink, receiver) = ChannelSink::new(1);
        drop(receiver);
        assert!(sink.publish(&sample_event()).await.is_err());
    }

    #[test]
    fn unix_socket_sink_rejects_empty_path() {
        assert!(UnixSocketSink::connect("").is_err());
    }

    #[cfg(not(unix))]
    #[tokio::test]
    async fn unix_socket_sink_is_unavailable_on_non_unix_platforms() {
        let error = UnixSocketSink::connect("vision.sock").expect_err("must be unavailable");
        assert!(matches!(error, VisionError::Unavailable(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_socket_sink_round_trips_newline_json() {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let directory = tempfile::tempdir().expect("temporary directory");
        let socket_path = directory.path().join("vision.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind test socket");
        let sink = UnixSocketSink::connect(&socket_path).expect("sink");
        assert_eq!(sink.path(), socket_path.as_path());

        let event = sample_event();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept connection");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read event line");
            line
        });

        sink.publish(&event).await.expect("publish event");
        let line = server.await.expect("server task");
        let parsed = VisionEvent::from_json_line(&line).expect("parse published event");
        assert_eq!(parsed, event);
        assert!(line.ends_with('\n'));
    }
}
