//! Command line entry point for the local camera tracking service.
//!
//! `hs-vision` is the standalone process form of the library: it captures
//! frames, tracks the person in front of the camera and publishes normalized
//! gaze events to the daemon over a local Unix socket. A synthetic source is
//! available for demos and development machines without a camera.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;

use hs_vision::{
    EventSink, FrameSource, SyntheticSource, UnixSocketSink, V4l2Source, VisionConfig, VisionError,
    VisionEvent, VisionResult, VisionService,
};

/// Maximum number of frames `--once` processes while waiting for an event.
const ONCE_MAX_FRAMES: usize = 600;

/// Local-first camera tracking service.
#[derive(Debug, Parser)]
#[command(
    name = "hs-vision",
    version,
    about = "Local camera tracking service: normalized gaze events over a Unix socket",
    long_about = None
)]
struct Cli {
    /// V4L2 capture device node (Linux only; ignored with --synthetic).
    #[arg(long, default_value = "/dev/video0")]
    device: String,

    /// Unix domain socket where newline-delimited JSON events are published.
    #[arg(long, default_value = "/run/homecore/vision.sock")]
    socket: PathBuf,

    /// Capture frame rate in frames per second.
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u32).range(1..=120))]
    fps: u32,

    /// Use the deterministic synthetic source instead of a real camera.
    #[arg(long)]
    synthetic: bool,

    /// Capture width in pixels.
    #[arg(long, default_value_t = 160, value_parser = clap::value_parser!(u32).range(1..=4096))]
    width: u32,

    /// Capture height in pixels.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(1..=4096))]
    height: u32,

    /// Process frames until the first event is published, then exit.
    #[arg(long)]
    once: bool,

    /// Print JSON events on stdout instead of writing to the Unix socket.
    #[arg(long)]
    stdout: bool,
}

/// Development sink that prints one JSON event per line on stdout.
struct StdoutSink;

#[async_trait::async_trait]
impl EventSink for StdoutSink {
    async fn publish(&self, event: &VisionEvent) -> VisionResult<()> {
        let line = event.to_json_line()?;
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(line.as_bytes()).map_err(VisionError::Io)?;
        handle.flush().map_err(VisionError::Io)?;
        Ok(())
    }
}

/// Initialise structured logging from `RUST_LOG`, defaulting to `info`.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "vision service failed");
            ExitCode::FAILURE
        }
    }
}

/// Build the configured ports and run the service.
async fn run(cli: Cli) -> VisionResult<()> {
    let config = VisionConfig {
        fps: cli.fps,
        ..VisionConfig::default()
    };

    let sink: Arc<dyn EventSink> = if cli.stdout {
        Arc::new(StdoutSink)
    } else {
        Arc::new(UnixSocketSink::connect(&cli.socket)?)
    };

    let source: Box<dyn FrameSource> = if cli.synthetic {
        Box::new(SyntheticSource::new(cli.width, cli.height)?.with_fps(cli.fps))
    } else {
        Box::new(V4l2Source::open(&cli.device, cli.width, cli.height)?)
    };

    let mut service = VisionService::new(source, sink, config)?;
    tracing::info!(
        camera = service.camera(),
        socket = %cli.socket.display(),
        fps = cli.fps,
        once = cli.once,
        stdout = cli.stdout,
        "vision service starting"
    );

    if cli.once {
        return run_until_first_event(&mut service).await;
    }

    let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = shutdown_sender.send(true);
        }
    });
    let result = service.run(shutdown_receiver).await;
    signal_task.abort();
    result
}

/// Process frames until the first event is published (used by `--once`).
async fn run_until_first_event(service: &mut VisionService) -> VisionResult<()> {
    for _ in 0..ONCE_MAX_FRAMES {
        match service.run_once().await {
            Ok(Some(event)) => {
                tracing::info!(kind = event.kind(), "vision event published");
                return Ok(());
            }
            Ok(None) => {}
            Err(VisionError::SourceExhausted) => {
                tracing::info!("frame source exhausted before a vision event was produced");
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }
    Err(VisionError::Source(format!(
        "no vision event was produced after {ONCE_MAX_FRAMES} frames"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_arguments_target_the_production_socket() {
        let cli = Cli::try_parse_from(["hs-vision"]).expect("default arguments parse");
        assert_eq!(cli.device, "/dev/video0");
        assert_eq!(cli.socket, PathBuf::from("/run/homecore/vision.sock"));
        assert_eq!(cli.fps, 15);
        assert_eq!(cli.width, 160);
        assert_eq!(cli.height, 120);
        assert!(!cli.synthetic);
        assert!(!cli.once);
        assert!(!cli.stdout);
    }

    #[test]
    fn synthetic_demo_arguments_parse() {
        let cli = Cli::try_parse_from([
            "hs-vision",
            "--synthetic",
            "--stdout",
            "--once",
            "--fps",
            "30",
            "--width",
            "320",
            "--height",
            "240",
        ])
        .expect("demo arguments parse");
        assert!(cli.synthetic);
        assert!(cli.stdout);
        assert!(cli.once);
        assert_eq!(cli.fps, 30);
        assert_eq!(cli.width, 320);
        assert_eq!(cli.height, 240);
    }

    #[test]
    fn out_of_range_frame_rate_is_rejected() {
        assert!(Cli::try_parse_from(["hs-vision", "--fps", "0"]).is_err());
        assert!(Cli::try_parse_from(["hs-vision", "--fps", "121"]).is_err());
        assert!(Cli::try_parse_from(["hs-vision", "--width", "0"]).is_err());
        assert!(Cli::try_parse_from(["hs-vision", "--height", "5000"]).is_err());
    }
}
