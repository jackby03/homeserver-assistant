# Local voice stack

`voice/` contains the optional **fully local** always-on voice listener. The
assistant answers when the household calls it by its configured name (default
`Aura`), and the name can be changed at any time in the portal without a restart
(`Asistente → Asistente de voz`).

## Why a helper instead of the browser

Browser speech APIs are typically cloud-backed (Chrome sends audio to Google),
which violates the appliance's local-first design. This helper keeps the audio
and the transcription on the machine:

```
microphone ──► faster-whisper (local STT) ──► POST /api/v1/kiosk/wake
                                                   │
                              daemon matches the configured name locally
                                                   │
                          kiosk listening window / agent query ──► speaker
```

Only local HTTP requests are made: `http://127.0.0.1:3000/api/v1/kiosk/*`,
authenticated with the kiosk token from `/var/lib/homecore/config.json`.

## Requirements

```bash
sudo apt install libportaudio2 ffmpeg
pip install -r voice/requirements.txt
```

`faster-whisper` downloads a model on first use. For an offline appliance,
pre-download it and enable offline mode:

```bash
python - <<'PY'
from faster_whisper import WhisperModel
WhisperModel("tiny", device="cpu", compute_type="int8")  # populates the local cache
PY
export HF_HUB_OFFLINE=1
```

Use a bigger model (`base`, `small`) for better accuracy on low-power hardware
trade-off. `tiny` runs comfortably on a laptop CPU.

## Run

```bash
# With the default config path and Spanish recognition
sudo python3 voice/wakeword.py --language es

# Development (daemon on a custom port, token from env)
HS_KIOSK_TOKEN=... python3 voice/wakeword.py --url http://127.0.0.1:8080 --once

# Optional local TTS (Piper): pipe the reply text to the model
python3 voice/wakeword.py --tts-command "piper --model /opt/piper/es_ES-sharvard-medium.onnx --output-raw | aplay -r 22050 -f S16_LE"
```

The helper prints the local transcripts and the assistant replies to the
console in English.

## Systemd unit (appliance)

```ini
[Unit]
Description=HomeServer Assistant local wake-word listener
After=hs-daemon.service sound.target
Wants=hs-daemon.service

[Service]
Type=simple
User=homeserver
SupplementaryGroups=audio
Environment=HF_HUB_OFFLINE=1
ExecStart=/usr/bin/python3 /opt/homeserver-assistant/voice/wakeword.py --language es
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

The helper needs read access to `/var/lib/homecore/config.json` (for the kiosk
token). Add the user to a group with read access or pass `--token`.

## Microphone hotword vs wake-word name matching

The wake word is **not** a fixed vendor model: the daemon normalizes the local
transcript, removes accents/case and matches the configured assistant name with
a small typo tolerance. Changing the name to `Casa`, `Jarvis` or `Oye Abuela`
works immediately for the matcher and the kiosk hint; the STT language is
configurable (`es`/`en`).

## Privacy guarantees

- Audio buffers live in RAM only and are discarded after transcription.
- Transcripts are sent only to the loopback daemon, which does not persist them.
- The wake-word matcher and the permission checks run in Rust on the appliance.
- No audio is written to disk, logged, or sent to any external service.
