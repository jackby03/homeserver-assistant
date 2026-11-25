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

## Wyoming voice stack (active)

The kiosk now speaks to the standard **Wyoming** stack used by Home Assistant
Voice. It runs as four local loopback services:

| Service | Port | Command |
|---|---|---|
| Wake word (Alkove, Spanish) | 10400 | `wyoming-alkove --uri tcp://127.0.0.1:10400 --lang es --word "hola aura" --any-word --models-dir /var/lib/hs-voice/wyoming/alkove` |
| Speech-to-text (faster-whisper) | 10300 | `wyoming-faster-whisper --uri tcp://127.0.0.1:10300 --data-dir /var/lib/hs-voice/wyoming/whisper --model base --language es --compute-type int8 --beam-size 1` |
| Text-to-speech (Piper) | 10200 | `wyoming-piper --uri tcp://127.0.0.1:10200 --data-dir /var/lib/hs-voice/wyoming/piper --voice es_ES-sharvard-medium` |
| Kiosk bridge | 5055 | `python3 voice/stt_server.py` |

`voice/stt_server.py` keeps the browser HTTP contract (`/wake`, `/transcribe`,
`/transcribe-fast`, `/speak`) and forwards each request to the corresponding
Wyoming service. Override the endpoints with `WYOMING_WAKE_URI`,
`WYOMING_ASR_URI` and `WYOMING_TTS_URI`.

If a Wyoming service is unavailable the bridge falls back to the local Vosk /
faster-whisper / Piper path, so the kiosk keeps working during upgrades. Point
the models at persistent storage (`/var/lib/hs-voice/wyoming`) on the
appliance; the development demo uses `.demo/wyoming`.

Windows development notes: `wyoming-piper` needs a small
`NotImplementedError` guard around `loop.add_signal_handler`, and the installed
PyAV 19 is newer than the version `faster-whisper` expects
(`metadata_errors` argument), so the demo venv has a local compatibility patch.
Neither patch is needed on the Linux appliance.

## Wake word fallback (Vosk)

Vosk remains as the offline fallback if the Wyoming wake service is down.

The browser/kiosk wake path uses **Vosk** keyword spotting, the same family of
engines used by Rhasspy. It runs a small Spanish model with a restricted grammar
(`["hola aura", "[unk]"]`) on 2 s clips, so Whisper is only used after the wake
word triggers. This is much more reliable for short names than transcribing idle
clips and matching text.

```bash
# 40 MB Spanish model (once)
curl -L -o vosk-es.zip https://alphacephei.com/vosk/models/vosk-model-small-es-0.42.zip
unzip vosk-es.zip -d /var/lib/homecore/models
export HS_VOSK_MODEL=/var/lib/homecore/models/vosk-model-small-es-0.42
python3 voice/stt_server.py
```

`POST /wake` accepts an audio body and returns
`{"wake": true, "text": "hola aura", "words": [...]}`. The kiosk auto-starts
continuous listening when Chrome already has microphone permission; the first
visit still requires the browser's one-time permission click.

Full open-source assistant stacks worth adopting instead of growing this
custom loop: Rhasspy, OpenVoiceOS, Willow and Home Assistant Voice (Wyoming).
They all compose wake word + VAD + ASR + intent + TTS; Vosk/openWakeWord and
Piper are the usual components. See `docs/ARCHITECTURE.md` for the appliance
boundary and keep the local Rust intent router as the fast path.

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

## Natural neural voice (Piper, recommended)

The appliance can speak with a local neural voice instead of the robotic SAPI
voices. Piper is small, CPU-only and runs fully offline (~77 MB voice model,
~200 MB RAM while synthesizing):

```bash
pip install piper-tts
# Spanish voice (download once from huggingface.co/rhasspy/piper-voices)
mkdir -p /opt/piper && cd /opt/piper
curl -L -O https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_ES/sharvard/medium/es_ES-sharvard-medium.onnx
curl -L -O https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_ES/sharvard/medium/es_ES-sharvard-medium.onnx.json

python voice/wakeword.py ... \
  --piper-exe piper \
  --piper-model /opt/piper/es_ES-sharvard-medium.onnx
```

With `--piper-exe/--piper-model` the helper synthesizes and plays the
"Te escucho" prompt and the assistant replies itself; without Piper it falls back
to the optional `--tts-command`, then to a short listening tone, while the kiosk
can speak replies with the browser's local voices.

Female Spanish voices in Piper: `es_MX-claude-high` (Mexico, female, high
quality) and `es_ES-mls_9972-low` (Spain, female, low). The tested demo voice is
`es_MX-claude-high` (`63 MB`, ~2 s synthesis on CPU).

## Low-latency wake detection

The listener does not wait for fixed 2.4 s windows. It streams 100 ms frames
with a small pre-roll, detects speech onset by RMS and transcribes as soon as the
user stops talking (0.45 s of trailing silence). The wake phrase uses beam size 1
while requests keep beam size 3, so the chime typically arrives ~1 s after
"Bober" instead of 3-4 s. Tune with `--wake-beam-size`, `--model` and the
device threshold; the log prints both the transcription time and the total flow.

## Continuous conversation

After the first wake word the helper keeps listening for follow-ups without
requiring the name again. A short tone marks that it is ready; each turn sends
the previous turns as history so the agent keeps context. It exits the
conversation after `--follow-up-seconds` (default 7 s) of silence, after
`--max-turns` (default 8) or when it hears "adiós"/"eso es todo"; use
`--no-conversation` to go back to one-shot behavior.

On this Windows demo the tested paths are:
`.demo\voice-venv\Scripts\piper.exe` and
`.demo\piper\es_ES-sharvard-medium.onnx` (these downloaded artifacts live under
the git-ignored `.demo/` directory).

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

## Kiosk push-to-talk (recommended on Windows)

The wall kiosk has a microphone button in the left chat panel. It records with
**Chrome's MediaRecorder** (the same Windows capture path as Audacity) and posts
the audio to a loopback STT server, so it does not depend on PortAudio's
DirectSound/MME/WASAPI stream quirks for the G435.

```bash
HF_HOME=/var/lib/homecore/hf python3 voice/stt_server.py
```

It listens on `127.0.0.1:5055`, decodes in memory with PyAV, transcribes with the
same faster-whisper `base` model and returns `{"text": "..."}`. It also exposes `POST /speak {"text": "..."}`, which renders the reply
with the bundled Piper female voice and returns `audio/wav`; the kiosk plays it
with an `<audio>` element instead of relying on the Windows SAPI voices. The kiosk then
sends that transcript through the normal local/LLM agent route. Audio is never
written to disk and leaves nothing but the loopback request.

On this Windows demo the server is already running:
`.\.demo\voice-venv\Scripts\python.exe voice\stt_server.py`.

### Continuous browser wake word

The kiosk chat panel has a small **waveform toggle** (next to the close button)
that starts continuous listening through Chrome. Every 2.5 s clip is transcribed
by the loopback server and sent to `/kiosk/wake`; when the configured name is
recognized the kiosk records the request and routes it through the normal
local/LLM agent. This uses the same WASAPI/default-microphone path as Audacity
and Chrome, avoiding the corrupt PortAudio/WDM capture on this host. The old
Python wake-word listener remains available for Linux appliances.

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

Say **“Bober”** followed by the request. The reply is printed and spoken with
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
a small typo tolerance. Changing the name to `Casa`, `Jarvis` or `Hola Abuela`
works immediately for the matcher and the kiosk hint; the STT language is
configurable (`es`/`en`).

## Privacy guarantees

- Audio buffers live in RAM only and are discarded after transcription.
- Transcripts are sent only to the loopback daemon, which does not persist them.
- The wake-word matcher and the permission checks run in Rust on the appliance.
- No audio is written to disk, logged, or sent to any external service.











