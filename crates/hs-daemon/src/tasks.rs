//! Background tasks: metrics sampling, vision bridge and session cleanup.

use std::sync::Arc;

use hs_core::events::NetworkStatus;
use hs_core::events::TrackingPoint;
use tokio::time::{interval, Duration, MissedTickBehavior};
use tracing::{debug, info, warn};

use crate::events::ServerEvent;
use crate::state::SharedState;
#[cfg(unix)]
use crate::vision_bridge::run_vision_bridge;

/// Spawn every background task required by the daemon.
pub fn spawn_all(state: SharedState) {
    spawn_metrics(state.clone());
    spawn_vision(state.clone());
    spawn_session_cleanup(state);
}

fn spawn_metrics(state: SharedState) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(3));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut connectivity_ok = false;
        let mut ticks: u64 = 0;
        loop {
            ticker.tick().await;
            ticks += 1;

            // Probe outbound connectivity every 15 seconds (5 ticks).
            if ticks % 5 == 0 {
                connectivity_ok = tokio::time::timeout(
                    Duration::from_secs(2),
                    tokio::net::TcpStream::connect("1.1.1.1:443"),
                )
                .await
                .map(|result| result.is_ok())
                .unwrap_or(false);
            }

            match state.system.overview().await {
                Ok(overview) => {
                    let online = overview
                        .network
                        .iter()
                        .any(|interface| !interface.ipv4.is_empty());
                    state.kiosk.set_network(NetworkStatus {
                        online,
                        dns_ok: connectivity_ok,
                        latency_ms: None,
                    });
                    if let Ok(value) = serde_json::to_value(&overview) {
                        state
                            .events
                            .publish_server(ServerEvent::Metrics { overview: value });
                    }
                }
                Err(err) => debug!(error = %err, "failed to sample system metrics"),
            }
        }
    });
}

fn spawn_vision(state: SharedState) {
    if state.daemon.dev_mode || state.daemon.fake_vision {
        info!("vision bridge running in synthetic mode");
        let kiosk = Arc::clone(&state.kiosk);
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(400));
            let start = std::time::Instant::now();
            let mut absent_until = None;
            loop {
                ticker.tick().await;
                let elapsed = start.elapsed().as_secs_f32();
                let cycle = elapsed % 20.0;
                if cycle < 6.0 {
                    if let Some(until) = absent_until {
                        if std::time::Instant::now() < until {
                            continue;
                        }
                        absent_until = None;
                    }
                    let x = 0.5 + 0.3 * (elapsed * 0.7).sin();
                    let y = 0.5 + 0.18 * (elapsed * 0.9).cos();
                    kiosk.set_tracking(Some(TrackingPoint {
                        x: x.clamp(0.0, 1.0),
                        y: y.clamp(0.0, 1.0),
                        confidence: 0.85,
                    }));
                } else if absent_until.is_none() {
                    kiosk.set_tracking(None);
                    absent_until = Some(std::time::Instant::now() + Duration::from_secs(14));
                }
            }
        });
        return;
    }

    #[cfg(unix)]
    {
        let socket = state.daemon.vision_socket.clone();
        let kiosk = Arc::clone(&state.kiosk);
        tokio::spawn(async move {
            if let Err(err) = run_vision_bridge(socket, kiosk).await {
                warn!(error = %err, "vision bridge stopped");
            }
        });
    }

    #[cfg(not(unix))]
    {
        debug!("vision bridge is only available on Unix platforms");
    }
}

fn spawn_session_cleanup(state: SharedState) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(3600));
        loop {
            ticker.tick().await;
            match state.auth.purge_expired_sessions() {
                Ok(removed) if removed > 0 => info!(removed, "purged expired sessions"),
                Ok(_) => {}
                Err(err) => warn!(error = %err, "failed to purge expired sessions"),
            }
        }
    });
}
