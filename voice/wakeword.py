#!/usr/bin/env python3
"""
Local always-on wake-word listener for HomeServer Assistant OS.

Privacy: audio is captured and transcribed **on the appliance**. Only the text
transcript is sent to the local daemon (127.0.0.1), which performs the wake-word
match against the configured assistant name. No audio or transcript leaves the
machine.

Pipeline:
  1. Capture short audio windows from the microphone (sounddevice).
  2. Transcribe them locally with faster-whisper (CTranslate2, CPU or GPU).
  3. POST the transcript to the kiosk wake endpoint. The daemon decides whether
     the assistant was called and returns `activated`.
  4. If activated, record the request until silence, transcribe it, POST it to
     the kiosk agent endpoint and print the reply (optionally pipe it to a local
     TTS command such as Piper).

The assistant name and sensitivity are configured in the web portal (Voice
assistant card); this helper does not need to know them.

Dependencies:
    pip install -r requirements.txt
The Whisper model is downloaded once by faster-whisper; pre-download it and set
HF_HUB_OFFLINE=1 for a fully offline appliance (see README.md).
"""
from __future__ import annotations

import argparse
import json
import os
import queue
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

try:
    import numpy as np
    import sounddevice as sd
    from faster_whisper import WhisperModel
except ImportError as error:  # pragma: no cover - dependency guidance
    print(
        f"missing local voice dependency: {error}\n"
        "install with: pip install -r voice/requirements.txt",
        file=sys.stderr,
    )
    raise SystemExit(2) from error

SAMPLE_RATE = 16_000
DEFAULT_CONFIG = Path("/var/lib/homecore/config.json")
DEFAULT_URL = "http://127.0.0.1:3000"
WAKE_WINDOW_SECONDS = 2.4
QUERY_MAX_SECONDS = 9.0
SILENCE_SECONDS = 1.2
DEFAULT_THRESHOLD = 0.004
DEFAULT_ASSISTANT_NAME = "Aura"


def read_kiosk_token(config_path: Path) -> str:
    """Read the kiosk token from the daemon config file (local, root-readable)."""
    env_token = os.environ.get("HS_KIOSK_TOKEN", "").strip()
    if env_token:
        return env_token
    try:
        data = json.loads(config_path.read_text(encoding="utf-8"))
        token = str(data.get("kiosk", {}).get("token", "")).strip()
        if token:
            return token
    except FileNotFoundError:
        pass
    except (OSError, ValueError) as error:
        print(f"[wakeword] cannot read {config_path}: {error}", file=sys.stderr)
    print(
        "[wakeword] no kiosk token found; pass --token or set HS_KIOSK_TOKEN",
        file=sys.stderr,
    )
    raise SystemExit(2)


def post_json(url: str, token: str, payload: dict) -> dict:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Content-Type": "application/json",
            "X-Kiosk-Token": token,
        },
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read().decode("utf-8")
    return json.loads(body) if body else {}


def get_json(url: str, token: str) -> dict:
    request = urllib.request.Request(url, headers={"X-Kiosk-Token": token})
    with urllib.request.urlopen(request, timeout=10) as response:
        body = response.read().decode("utf-8")
    return json.loads(body) if body else {}


def fetch_assistant_name(url: str, token: str) -> str:
    """Read the configured assistant name from the kiosk state."""
    try:
        state = get_json(f"{url}/api/v1/kiosk/state", token)
        name = str(state.get("assistant_name", "")).strip()
        if name:
            return name
    except (urllib.error.URLError, TimeoutError, ValueError, OSError):
        pass
    return DEFAULT_ASSISTANT_NAME


def input_devices() -> list[tuple[int, dict]]:
    return [
        (index, device)
        for index, device in enumerate(sd.query_devices())
        if device["max_input_channels"] > 0
    ]


def print_input_devices() -> None:
    print("[wakeword] input devices:")
    for index, device in input_devices():
        print(f"  [{index:2d}] {device['name']} (host API {device['hostapi']})")


def sample_level(device: int, seconds: float = 0.5) -> float | None:
    try:
        audio = sd.rec(
            int(seconds * SAMPLE_RATE),
            samplerate=SAMPLE_RATE,
            channels=1,
            device=device,
            dtype="float32",
        )
        sd.wait()
        return rms(audio.flatten())
    except Exception:  # unsupported host API/sample rate: skip this device
        return None


def choose_input_device() -> tuple[int, float] | None:
    """Pick the input device with the strongest live signal."""
    best: tuple[float, int] | None = None
    for index, device in input_devices():
        level = sample_level(index)
        if level is None:
            continue
        print(f"[wakeword] input [{index:2d}] level={level:.6f} {device['name']}")
        if level > 0.0005 and (best is None or level > best[0]):
            best = (level, index)
    if best is None:
        return None
    print(f"[wakeword] auto-selected input device [{best[1]}]")
    return best[1], best[0]


def rms(block: np.ndarray) -> float:
    if block.size == 0:
        return 0.0
    return float(np.sqrt(np.mean(np.square(block, dtype=np.float64))))


def record_window(seconds: float, device: str | int | None) -> np.ndarray:
    frames = int(seconds * SAMPLE_RATE)
    audio = sd.rec(frames, samplerate=SAMPLE_RATE, channels=1, dtype="float32", device=device)
    sd.wait()
    return audio.flatten()


def record_until_silence(
    max_seconds: float,
    silence_seconds: float,
    threshold: float,
    device: str | int | None,
) -> np.ndarray:
    """Record the user's request, stopping after `silence_seconds` of quiet.

    Uses short blocking ``sd.rec`` windows instead of ``InputStream.read`` so it
    also works on WDM-KS devices, which reject the blocking read API on Windows.
    """
    chunk = 0.1
    blocks: list[np.ndarray] = []
    silent_for = 0.0
    speech_started = False
    deadline = time.monotonic() + max_seconds
    while time.monotonic() < deadline:
        block = sd.rec(
            int(chunk * SAMPLE_RATE),
            samplerate=SAMPLE_RATE,
            channels=1,
            dtype="float32",
            device=device,
        )
        sd.wait()
        mono = block.flatten()
        blocks.append(mono.copy())
        if rms(mono) < threshold:
            if speech_started:
                silent_for += chunk
                if silent_for >= silence_seconds:
                    break
            # Before the user starts talking, keep waiting instead of stopping
            # on the natural pause that follows the wake word.
        else:
            speech_started = True
            silent_for = 0.0
    return np.concatenate(blocks) if blocks else np.zeros(0, dtype="float32")


def normalize_audio(audio: np.ndarray) -> np.ndarray:
    """Lift quiet microphones to a usable level (max 5x, clipped)."""
    if audio.size == 0:
        return audio
    level = rms(audio)
    if level <= 0.0:
        return audio
    gain = min(0.06 / level, 5.0)
    return np.clip(audio * gain, -1.0, 1.0)


def transcribe(
    model: WhisperModel,
    audio: np.ndarray,
    language: str,
    initial_prompt: str | None = None,
    beam_size: int = 3,
) -> str:
    """Transcribe a window locally; model/driver failures must not kill the loop."""
    try:
        audio = normalize_audio(audio)
        segments, _info = model.transcribe(
            audio,
            language=language if language in {"es", "en"} else None,
            beam_size=beam_size,
            vad_filter=True,
            condition_on_previous_text=False,
            initial_prompt=initial_prompt,
        )
        return " ".join(segment.text.strip() for segment in segments).strip()
    except Exception as error:  # local model/driver failure: keep listening
        print(f"[wakeword] transcription failed: {error}", file=sys.stderr)
        return ""


def speak(command: str | None, text: str) -> None:
    """Pipe the reply to an optional local TTS command (e.g. piper)."""
    if not command or not text:
        return
    import subprocess  # local only

    try:
        subprocess.run(  # noqa: S603 - operator supplied local command
            command,
            input=text,
            text=True,
            shell=True,
            check=False,
            timeout=60,
        )
    except Exception as error:  # TTS must never kill the microphone loop
        print(f"[wakeword] TTS failed: {error}", file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser(description="Local wake-word listener")
    parser.add_argument("--url", default=DEFAULT_URL, help="local kiosk base URL")
    parser.add_argument("--token", default="", help="kiosk token (default: read config)")
    parser.add_argument(
        "--config", type=Path, default=DEFAULT_CONFIG, help="daemon config.json"
    )
    parser.add_argument(
        "--model",
        default="base",
        help="faster-whisper model (tiny|base|small); base balances accuracy and memory",
    )
    parser.add_argument("--language", default="es", help="STT language (es/en)")
    parser.add_argument(
        "--stt-device",
        default="cpu",
        choices=["cpu", "cuda", "auto"],
        help="inference device for the local model (default: cpu)",
    )
    parser.add_argument(
        "--device",
        default=None,
        type=lambda value: int(value) if value.isdigit() else value,
        help="audio input device index/name (default: system default)",
    )
    parser.add_argument(
        "--auto-input",
        action="store_true",
        help="probe all microphones and use the one with the strongest signal",
    )
    parser.add_argument(
        "--list-devices", action="store_true", help="list microphones and exit"
    )
    parser.add_argument(
        "--calibrate",
        action="store_true",
        help="measure 3 seconds of microphone level and exit",
    )
    parser.add_argument(
        "--assistant-name",
        default="",
        help="wake name (default: read from the kiosk state)",
    )
    parser.add_argument(
        "--initial-prompt",
        default="",
        help="Whisper prompt bias; defaults to the assistant name",
    )
    parser.add_argument(
        "--beam-size", type=int, default=3, help="Whisper beam size"
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=DEFAULT_THRESHOLD,
        help="RMS silence threshold",
    )
    parser.add_argument("--once", action="store_true", help="process a single window and exit")
    parser.add_argument(
        "--tts-command",
        default="",
        help="optional local TTS command reading text on stdin (e.g. piper --model ...)",
    )
    args = parser.parse_args()

    if args.list_devices:
        print_input_devices()
        return 0

    device = args.device
    if args.calibrate:
        level = rms(record_window(3.0, device))
        print(f"[wakeword] microphone RMS over 3 s: {level:.6f}")
        print(f"[wakeword] suggested threshold: {max(0.002, level * 0.4):.6f}")
        return 0
    if args.auto_input and device is None:
        choice = choose_input_device()
        if choice is not None:
            device, ambient = choice
            # One extra stream open would hang on WDM-KS; reuse the sample that
            # auto-selection already collected. Cap it so a loud setup moment
            # cannot make the assistant deaf during normal speech.
            args.threshold = max(DEFAULT_THRESHOLD, min(ambient * 1.1, 0.009))
            print(
                f"[wakeword] ambient={ambient:.6f} "
                f"threshold={args.threshold:.4f}"
            )

    token = args.token.strip() or read_kiosk_token(args.config)
    assistant_name = args.assistant_name.strip() or fetch_assistant_name(
        args.url, token
    )
    initial_prompt = args.initial_prompt.strip() or (
        f"Oye {assistant_name}. Hola {assistant_name}."
    )
    try:
        input_name = sd.query_devices(device)["name"] if device is not None else sd.query_devices(kind="input")["name"]
    except Exception:
        input_name = str(device)
    print(
        f"[wakeword] assistant='{assistant_name}' input='{input_name}' "
        f"threshold={args.threshold:.4f}"
    )
    print(f"[wakeword] loading local STT model '{args.model}' (first run downloads it)")
    # CPU is the safe default: CUDA builds need cublas/cuDNN libraries that an
    # appliance (or this Windows host) may not have. Use --stt-device cuda to
    # opt into GPU inference when the runtime is installed.
    model = WhisperModel(args.model, device=args.stt_device, compute_type="int8")
    print(f"[wakeword] listening locally; wake endpoint {args.url}/api/v1/kiosk/wake")

    last_heartbeat = 0.0
    silent_windows = 0
    while True:
        # Tell the daemon that a local microphone listener is alive; the portal
        # shows "listener connected" only while these heartbeats arrive.
        now = time.monotonic()
        if now - last_heartbeat >= 15.0:
            try:
                post_json(
                    f"{args.url}/api/v1/kiosk/voice/heartbeat", token, {}
                )
            except (urllib.error.URLError, TimeoutError, ValueError):
                pass
            last_heartbeat = now

        window = record_window(WAKE_WINDOW_SECONDS, device)
        level = rms(window)
        if level < args.threshold:
            silent_windows += 1
            if silent_windows == 1 or silent_windows % 30 == 0:
                print(
                    f"[wakeword] level={level:.6f} below threshold "
                    f"({args.threshold:.4f}); waiting for speech"
                )
            if args.once:
                print("[wakeword] silence; no wake word detected")
                return 0
            continue
        silent_windows = 0
        started = time.monotonic()
        transcript = transcribe(
            model, window, args.language, initial_prompt, args.beam_size
        )
        elapsed = time.monotonic() - started
        if transcript:
            print(f"[wakeword] heard ({elapsed:.2f}s): {transcript}")
        else:
            if args.once:
                print("[wakeword] no speech detected")
                return 0
            continue
        try:
            outcome = post_json(
                f"{args.url}/api/v1/kiosk/wake",
                token,
                {"transcript": transcript, "confidence": None},
            )
        except (urllib.error.URLError, TimeoutError, ValueError) as error:
            print(f"[wakeword] daemon unreachable: {error}", file=sys.stderr)
            time.sleep(2)
            if args.once:
                return 1
            continue

        if not outcome.get("activated"):
            if args.once:
                print(f"[wakeword] not activated: {outcome.get('reason')}")
                return 0
            continue

        print(f"[wakeword] activated by '{outcome.get('assistant_name')}'")
        trailing = (outcome.get("trailing_query") or "").strip()
        query = trailing
        if not query:
            # Let the user know the assistant is listening for the request; the
            # prompt plays in their headset, not through room speakers.
            speak(
                args.tts_command or None,
                "Te escucho." if args.language == "es" else "I'm listening.",
            )
            try:
                request_audio = record_until_silence(
                    QUERY_MAX_SECONDS, SILENCE_SECONDS, args.threshold, device
                )
            except Exception as error:  # device/driver hiccup: fixed fallback
                print(
                    f"[wakeword] request recording failed ({error}); "
                    "recording a fixed window instead",
                    file=sys.stderr,
                )
                request_audio = record_window(5.0, device)
            query = transcribe(model, request_audio, args.language, beam_size=args.beam_size)
        if not query:
            print("[wakeword] no request after the wake word")
            if args.once:
                return 0
            continue

        print(f"[wakeword] request: {query}")
        try:
            response = post_json(
                f"{args.url}/api/v1/kiosk/agent/query",
                token,
                # The helper speaks the reply itself; do not duplicate it in
                # the kiosk when the query came from the microphone helper.
                {"text": query, "speak": False},
            )
        except (urllib.error.URLError, TimeoutError, ValueError) as error:
            print(f"[wakeword] agent query failed: {error}", file=sys.stderr)
            if args.once:
                return 1
            continue

        reply = str(response.get("reply", "")).strip()
        print(f"[wakeword] reply: {reply}")
        speak(args.tts_command or None, reply)
        if args.once:
            return 0


if __name__ == "__main__":
    raise SystemExit(main())






