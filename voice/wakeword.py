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
WAKE_WINDOW_SECONDS = 1.8
QUERY_MAX_SECONDS = 9.0
SILENCE_SECONDS = 1.2


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


def rms(block: np.ndarray) -> float:
    if block.size == 0:
        return 0.0
    return float(np.sqrt(np.mean(np.square(block, dtype=np.float64))))


def record_window(seconds: float, device: str | None) -> np.ndarray:
    frames = int(seconds * SAMPLE_RATE)
    audio = sd.rec(frames, samplerate=SAMPLE_RATE, channels=1, dtype="float32", device=device)
    sd.wait()
    return audio.flatten()


def record_until_silence(
    max_seconds: float, silence_seconds: float, threshold: float, device: str | None
) -> np.ndarray:
    """Record the user's request, stopping after `silence_seconds` of quiet."""
    chunk = 0.1
    blocks: list[np.ndarray] = []
    silent_for = 0.0
    deadline = time.monotonic() + max_seconds
    with sd.InputStream(
        samplerate=SAMPLE_RATE, channels=1, dtype="float32", device=device
    ) as stream:
        while time.monotonic() < deadline:
            block, _overflow = stream.read(int(chunk * SAMPLE_RATE))
            mono = block.flatten()
            blocks.append(mono.copy())
            if rms(mono) < threshold:
                silent_for += chunk
                if silent_for >= silence_seconds and len(blocks) > 10:
                    break
            else:
                silent_for = 0.0
    return np.concatenate(blocks) if blocks else np.zeros(0, dtype="float32")


def transcribe(model: WhisperModel, audio: np.ndarray, language: str) -> str:
    segments, _info = model.transcribe(
        audio,
        language=language if language in {"es", "en"} else None,
        beam_size=1,
        vad_filter=True,
        condition_on_previous_text=False,
    )
    return " ".join(segment.text.strip() for segment in segments).strip()


def speak(command: str | None, text: str) -> None:
    """Pipe the reply to an optional local TTS command (e.g. piper)."""
    if not command or not text:
        return
    import subprocess  # local only

    try:
        subprocess.run(  # noqa: S603 - operator supplied local command
            command,
            input=text,
            shell=True,
            check=False,
            timeout=60,
        )
    except (OSError, subprocess.SubprocessError) as error:
        print(f"[wakeword] TTS failed: {error}", file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser(description="Local wake-word listener")
    parser.add_argument("--url", default=DEFAULT_URL, help="local kiosk base URL")
    parser.add_argument("--token", default="", help="kiosk token (default: read config)")
    parser.add_argument(
        "--config", type=Path, default=DEFAULT_CONFIG, help="daemon config.json"
    )
    parser.add_argument("--model", default="tiny", help="faster-whisper model name/path")
    parser.add_argument("--language", default="es", help="STT language (es/en)")
    parser.add_argument("--device", default=None, help="audio input device")
    parser.add_argument(
        "--threshold", type=float, default=0.012, help="RMS silence threshold"
    )
    parser.add_argument("--once", action="store_true", help="process a single window and exit")
    parser.add_argument(
        "--tts-command",
        default="",
        help="optional local TTS command reading text on stdin (e.g. piper --model ...)",
    )
    args = parser.parse_args()

    token = args.token.strip() or read_kiosk_token(args.config)
    print(f"[wakeword] loading local STT model '{args.model}' (first run downloads it)")
    model = WhisperModel(args.model, device="auto", compute_type="int8")
    print(f"[wakeword] listening locally; wake endpoint {args.url}/api/v1/kiosk/wake")

    while True:
        window = record_window(WAKE_WINDOW_SECONDS, args.device)
        if rms(window) < args.threshold:
            if args.once:
                print("[wakeword] silence; no wake word detected")
                return 0
            continue
        transcript = transcribe(model, window, args.language)
        if transcript:
            print(f"[wakeword] heard: {transcript}")
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
            request_audio = record_until_silence(
                QUERY_MAX_SECONDS, SILENCE_SECONDS, args.threshold, args.device
            )
            query = transcribe(model, request_audio, args.language)
        if not query:
            print("[wakeword] no request after the wake word")
            if args.once:
                return 0
            continue

        print(f"[wakeword] request: {query}")
        try:
            response = post_json(
                f"{args.url}/api/v1/kiosk/agent/query", token, {"text": query}
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
