# hs-vision — local camera tracking

`hs-vision` is the local-first camera tracking service of HomeServer Assistant
OS. It looks at a camera, finds the person in front of it and publishes
**normalized gaze coordinates** to the daemon. Everything runs on the appliance:
no cloud calls, no model downloads, no Python requirement for tracking.

It is deliberately small:

* **Rust** — frame acquisition, motion detection, event publishing and the
  service loop.
* **Optional Python** — a helper that produces face embeddings *only* during
  profile enrollment, using InsightFace/ONNX. Not used at runtime.

```
┌──────────────┐   Frame (gray)   ┌───────────────┐   VisionEvent   ┌──────────────────┐
│  V4L2 camera │ ───────────────▶ │    Detector   │ ──────────────▶ │  EventSink       │
│  /dev/video0 │                  │ background +  │                 │ UnixSocketSink   │
└──────────────┘                  │ centroid +EMA │                 │ ChannelSink      │
       ▲                          └───────────────┘                 │ NullSink         │
       │                           VisionService:                    └────────┬─────────┘
       │                           pacing, presence                           │ newline JSON
       │                           debounce, heartbeat                      ▼
┌──────────────┐                                                   /run/homecore/vision.sock
│ SyntheticSrc │  demos + tests                                          │
└──────────────┘                                                          ▼
                                                                   hs-daemon → KioskEvent
```

## Layout

| Path | Purpose |
|---|---|
| `crates/hs-vision/src/frame.rs` | `Frame` + `FrameSource` port |
| `crates/hs-vision/src/synthetic.rs` | Deterministic synthetic camera |
| `crates/hs-vision/src/v4l2/` | Linux V4L2 capture source (stub elsewhere) |
| `crates/hs-vision/src/detector.rs` | Pure-Rust background-subtraction detector |
| `crates/hs-vision/src/event.rs` | Newline-delimited JSON wire format |
| `crates/hs-vision/src/sink.rs` | `EventSink`, `UnixSocketSink`, `ChannelSink`, `NullSink` |
| `crates/hs-vision/src/service.rs` | `VisionService` / `VisionConfig` loop |
| `crates/hs-vision/src/main.rs` | `hs-vision` binary (clap CLI) |
| `vision/python/face_embed.py` | Optional enrollment embedding helper |

## Wire format

The service writes **one JSON object per line** (newline-delimited JSON, UTF-8)
to a local Unix domain socket. Timestamps are RFC 3339 UTC.

Tracking — a person is present; coordinates are normalized, `x` left→right and
`y` top→bottom, both in `[0, 1]`, `confidence` in `[0, 1]`:

```json
{"type":"tracking","point":{"x":0.42,"y":0.55,"confidence":0.87},"timestamp":"2026-01-02T03:04:05Z"}
```

Absence — nobody has been tracked for `absent_after_frames` consecutive frames.
Emitted once per transition, not repeatedly:

```json
{"type":"absence","timestamp":"2026-01-02T03:04:06Z"}
```

Heartbeat — liveness/diagnostics every 10 seconds while running:

```json
{"type":"heartbeat","camera":"/dev/video0","fps":15,"timestamp":"2026-01-02T03:04:15Z"}
```

The daemon maps events to the kiosk model:

| Vision event | Daemon action |
|---|---|
| `tracking` | `KioskEvent::Tracking { point: Some(point.into()) }` with `hs_core::events::TrackingPoint` |
| `absence` | `KioskEvent::Tracking { point: None }` |
| `heartbeat` | Not forwarded; liveness bookkeeping only |

`VisionEvent` lives in `crates/hs-vision/src/event.rs`. It intentionally defines
its own `TrackingPoint` so the socket contract is owned by the vision service;
the conversion into `hs_core::events::TrackingPoint` is a `From` implementation.

## Privacy design

* **Frames never leave the device.** Grayscale frames live in memory only; they
  are not written to disk, logged, or sent anywhere. Only normalized coordinates
  (floats) are published.
* **No cloud, no telemetry.** The service has no network code. The Unix socket
  is a local filesystem path; there is no TCP listener.
* **Biometrics stay local.** Face embeddings are produced during enrollment by
  an optional local helper and stored encrypted via
  `hs_core::state::SecretVault`. Matching happens in Rust (`hs-profiles`);
  embeddings are never logged (`AGENTS.md` golden rule 2).
* **Least privilege.** Run the service as an unprivileged user in the `video`
  group; it only needs read access to the camera node and write access to the
  socket path.
* **No identity from tracking.** Tracking is presence/gaze only. It never
  identifies a person; that is a separate, explicit enrollment + matching flow.

## Building and running

```bash
# Build
cargo build --offline -p hs-vision

# Synthetic demo (no camera, no daemon; prints events on stdout)
cargo run --offline -p hs-vision -- --synthetic --stdout

# One event and exit (useful for smoke tests)
cargo run --offline -p hs-vision -- --synthetic --stdout --once

# Production: real camera, daemon socket
cargo run --offline -p hs-vision --release -- \
  --device /dev/video0 --socket /run/homecore/vision.sock --fps 15

# CLI help
cargo run --offline -p hs-vision -- --help
```

| Flag | Default | Meaning |
|---|---|---|
| `--device` | `/dev/video0` | V4L2 device node (Linux); ignored with `--synthetic` |
| `--socket` | `/run/homecore/vision.sock` | Unix socket that `hs-daemon` listens on |
| `--fps` | `15` | Frame rate, `1..=120` |
| `--synthetic` | off | Deterministic moving-target source (no hardware) |
| `--width` / `--height` | `160` / `120` | Requested capture resolution, `1..=4096` |
| `--once` | off | Process until the first event is published, then exit |
| `--stdout` | off | Print JSON events to stdout instead of the socket |

Exit codes: `0` success (including `--once` finding no event), `1` runtime
error (camera/socket failure), `2` invalid arguments (clap).

Logging uses `tracing`; set `RUST_LOG=debug` for per-event logs.

### Daemon wiring

* `hs-daemon` binds the Unix domain socket **before or after** the service
  starts: `UnixSocketSink` connects lazily and reconnects after the daemon
  restarts, so startup order does not matter.
* Recommended permissions: directory `/run/homecore` owned by
  `homecore:homecore` with mode `0750`, socket mode `0660`, daemon and vision
  service running in the same group.
* Read the socket line by line, split on `\n`, parse JSON into a mirrored
  `{"type": "...", ...}` enum and forward `tracking`/`absence` to the kiosk.
* In development tests, use `ChannelSink::new(capacity)` and receive
  `VisionEvent`s directly instead of touching the filesystem.

Example systemd unit (the packaged unit lives under `infra/`):

```ini
[Unit]
Description=HomeServer Assistant local vision tracking
After=multi-user.target

[Service]
Type=simple
User=homecore-vision
SupplementaryGroups=video
ExecStart=/usr/lib/homecore/hs-vision --device /dev/video0 --socket /run/homecore/vision.sock --fps 15
Restart=on-failure
RestartSec=2
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

## Architecture notes

### Frame sources

* `SyntheticSource` renders a 160×120 (configurable) textured room with a bright
  rectangular "person" that walks in after a five-frame empty-room warmup and
  then moves along two sinusoids. It is deterministic, so tests compare frames
  byte-for-byte. `with_absent_after(n)` makes the person leave the scene, which
  exercises tracking→absence transitions end-to-end.
* `V4l2Source` (Linux) opens a V4L2 device, negotiates packed YUYV at the
  requested size and reads frames with the `read(2)` capture interface, then
  converts YUYV→luma. Non-Linux builds expose a stub whose `open()` returns
  `VisionError::Unavailable`, keeping Windows/macOS development and CI green.

### Detector

1. The first frame seeds a running-average background model.
2. Each pixel whose absolute luminance difference from the background exceeds
   `diff_threshold` is foreground; other pixels slowly adapt to the current
   frame (`learn_rate`).
3. Foreground pixels are aggregated into a bounding box and centroid. Blobs
   smaller than `min_area_fraction` are rejected as noise. If more than
   `max_area_fraction` of the frame changes at once, it is treated as an
   illumination change: the background is rebuilt and no detection is reported.
4. The centroid is normalized to `[0, 1]` and smoothed with an exponential
   moving average (`smoothing`). Confidence combines blob area and contrast.
5. `VisionService` applies `min_confidence`; below it the frame counts as a
   miss. After `absent_after_frames` consecutive misses an `absence` event is
   published once.

`Detector` is a pure function of the stream: no I/O, no threads, no clock. That
keeps it trivially testable with handcrafted frames.

### Service loop

* Ticks with a Tokio interval at `fps`.
* Presence transitions are published immediately. Steady-state tracking events
  are refreshed at roughly `fps / 10` frames (at 15 fps that is every frame, so
  the avatar's gaze follows smoothly).
* Heartbeat every 10 seconds.
* Graceful shutdown via `tokio::sync::watch<bool>` (Ctrl-C in the binary);
  frame-source exhaustion is a clean stop, and repeated I/O errors abort after
  50 consecutive failures.

## Tuning

| Knob | Default | Effect |
|---|---|---|
| `fps` (`--fps`) | `15` | CPU load vs. responsiveness; 10–20 is plenty for gaze |
| `VisionConfig::smoothing` | `0.35` | Lower = smoother but laggier gaze; `1.0` = raw centroid |
| `VisionConfig::min_confidence` | `0.35` | Raise to ignore small/dim blobs, lower in dim rooms |
| `VisionConfig::absent_after_frames` | `15` | About 1 s at 15 fps; raise to avoid flicker |
| `DetectorConfig::diff_threshold` | `25` | Raise for noisy sensors, lower for low contrast |
| `DetectorConfig::learn_rate` | `0.05` | Background adaptation speed |
| `DetectorConfig::min_area_fraction` | `0.005` | Minimum blob size; raise for noisy scenes |
| `DetectorConfig::max_area_fraction` | `0.6` | Above this, treat the frame as an illumination change |

Use `VisionConfig::default()` / `DetectorConfig::default()` and override fields
with `..Default::default()`.

## Limitations

* **Background subtraction assumptions.** The camera must be static and lighting
  reasonably stable. A person standing still stays detected (foreground pixels
  do not adapt), but an object moved into the scene is reported as foreground
  until restart, and rapid light changes briefly suppress detection while the
  background is rebuilt. There is no deep-learning person model.
* **Startup background.** The first frame seeds the background model. Start the
  service while the room is empty (the recommended appliance boot sequence); a
  person standing in view during the first frame becomes a persistent foreground
  ghost that biases the centroid until the service restarts. A robust
  multi-frame median warmup is a planned improvement.
* **Single person.** The centroid merges all foreground blobs; two people are
  reported as their average position.
* **Read I/O only (V4L2).** Capture uses the legacy `read(2)` interface because
  it needs no buffer management. Some UVC webcams only expose the streaming
  (MMAP) interface; those devices fail with a clear `Unavailable` error. This is
  a known gap: adding MMAP capture is a future improvement and does not change
  the public API.
* **No identity.** Tracking knows "someone is there", not "who". Identity comes
  from enrollment + local embedding matching.
* **No audio, no recording, no snapshots** are produced by this service.

## Optional Python face-embedding helper

Only needed when enrolling a family profile from a still image. Rust remains the
owner of matching and thresholds.

```bash
# One-time provisioning on a trusted machine (needs network):
python3 -m venv /opt/homecore/vision-venv
/opt/homecore/vision-venv/bin/pip install -r vision/python/requirements.txt
# InsightFace downloads model weights under ~/.insightface on first use;
# do that during provisioning, then copy the directory to the appliance.

# Runtime use (no network):
/opt/homecore/vision-venv/bin/python vision/python/face_embed.py --image /tmp/enroll.jpg
cat /tmp/enroll.jpg | /opt/homecore/vision-venv/bin/python vision/python/face_embed.py --stdin
```

Output on stdout (one JSON object):

```json
{"embedding":[0.012345,...],"model":"buffalo_l","dim":512}
```

* The helper never performs network I/O itself. If the model directory
  (`$INSIGHTFACE_HOME/models/<model>`, default `~/.insightface`) is missing it
  exits with code `3` unless `--allow-download` is passed, which should only be
  used on a provisioning machine.
* The embedding is the normalized (`normed_embedding`) representation of the
  largest detected face, rounded to 6 decimals.
* Errors go to stderr as `{"error": "..."}` (codes: `2` invalid image, `3`
  missing dependency/model, `4` extraction failure such as no face detected).
* Store embeddings encrypted (`SecretVault`) and compare them in Rust with
  constant-time-friendly cosine similarity; never log embeddings or images.

## Testing

```bash
cargo test --offline -p hs-vision --lib --tests
cargo clippy --offline -p hs-vision --all-targets -- -D warnings
```

The suite is hermetic: no camera, root, network, Python or sleeping is required.
It covers frame validation, YUYV→luma conversion, detector behaviour (known
positions, absence, noise rejection, illumination changes, smoothing, confidence
bounds), synthetic-source determinism and absence, exact wire JSON, sink
delivery (including a real Unix socket round-trip on Unix), service transitions
and graceful shutdown, plus CLI argument parsing.
