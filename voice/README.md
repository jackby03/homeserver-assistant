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
authenticated with the kiosk token from `/var/lib/homecore/config.json`. The
helper also posts a heartbeat every 15 s so the portal can show whether the
microphone listener is connected.

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

## Model size guide (local, int8 CPU)

| Model | Disk | RAM (approx.) | Accuracy / speed |
|---|---|---|---|
| `tiny` | ~75 MB | ~150 MB | fastest, weaker with short names |
| `base` (default) | ~145 MB | ~250 MB | good Spanish, real-time on a laptop CPU |
| `small` | ~466 MB | ~600 MB | better accuracy, slower (~2-3× base) |

The default is `base` with `beam_size=3`, VAD and an initial prompt containing
the assistant name, which reduces the "Aura -> Laura/ahora" errors. Larger
options such as Qwen3-ASR, NVIDIA Nemotron Speech or CosyVoice/Fish are heavier
(1-4 GB plus extra runtimes) and are not needed for a low-memory appliance;
CosyVoice/Fish are TTS models, not ASR. Use `--model small` when accuracy matters
more than CPU, and `--model tiny` on very weak hardware.

Useful input diagnostics:

```powershell
python voice/wakeword.py --list-devices
python voice/wakeword.py --calibrate --device 22
python voice/wakeword.py --auto-input --model base   # pick the strongest mic
```

`--auto-input` samples every microphone, selects the one with the strongest
signal and sets the RMS threshold safely above its noise floor.

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

## Windows development demo (local microphone)

On a Windows development machine the same helper runs against the dev daemon:

```powershell
python -m venv .demo\voice-venv
.\.demo\voice-venv\Scripts\pip install -r voice\requirements.txt

# List capture devices and pick one with "Microphone" / "Headset"
.\.demo\voice-venv\Scripts\python -c "import sounddevice as sd; print(sd.query_devices())"

.\.demo\voice-venv\Scripts\python voice\wakeword.py `
  --url http://127.0.0.1:3000 `
  --config .demo\data\config.json `
  --language es `
  --tts-command "powershell.exe -NoProfile -ExecutionPolicy Bypass -File voice\tts.ps1"
```

Say **“Oye Aura”** followed by the request. The reply is printed and spoken with
the local Spanish SAPI voice; no audio or text leaves the machine. The portal's
voice card shows **“Micrófono local conectado”** while the helper heartbeats (it
sends one every 15 s and the daemon considers it connected for 45 s).

If Windows has a system proxy configured (WinINET), pip may fail with
`ProxyError ... 127.0.0.1:8000`. Bypass it for the install only:

```powershell
$env:NO_PROXY='*'; .\.demo\voice-venv\Scripts\pip install --proxy "" -r voice\requirements.txt
```

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


