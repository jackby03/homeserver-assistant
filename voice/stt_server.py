#!/usr/bin/env python3
"""Loopback STT server for the kiosk push-to-talk microphone.

The kiosk (Chrome) records the user with `MediaRecorder`, which uses the same
Windows audio capture path as Audacity and works with the G435 when PortAudio's
DirectSound/MME streams are corrupt. This tiny local server decodes the uploaded
audio with PyAV and transcribes it with the same faster-whisper model as the
wake-word helper.

Privacy: binds to 127.0.0.1 only; audio is decoded in memory and never written to
disk; transcripts are returned to the local kiosk page.
"""
from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import tempfile
import threading
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import av
import numpy as np
from vosk import KaldiRecognizer, Model

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
import wakeword  # noqa: E402

HOST = "127.0.0.1"
PORT = 5055
ALLOWED_ORIGINS = {
    "http://127.0.0.1:3000",
    "http://localhost:3000",
    "http://127.0.0.1:5173",
    "http://localhost:5173",
}
MODEL = wakeword.WhisperModel("base", device="cpu", compute_type="int8")

VOSK_MODEL_PATH = Path(
    os.environ.get(
        "HS_VOSK_MODEL", str(ROOT.parent / ".demo" / "vosk-model-small-es-0.42")
    )
)
VOSK_MODEL: Model | None = None
if VOSK_MODEL_PATH.is_dir():
    try:
        VOSK_MODEL = Model(str(VOSK_MODEL_PATH))
        print(f"vosk wake model loaded from {VOSK_MODEL_PATH}")
    except Exception as error:
        print(f"vosk model failed to load: {error}", file=sys.stderr)
else:
    print(f"vosk model not found at {VOSK_MODEL_PATH}", file=sys.stderr)

WAKE_GRAMMAR = '["hola aura", "[unk]"]'

PIPER_EXE = Path(
    os.environ.get("HS_PIPER_EXE", str(Path(sys.executable).with_name("piper.exe")))
)
PIPER_MODEL = Path(
    os.environ.get(
        "HS_PIPER_MODEL",
        str(ROOT.parent / ".demo" / "piper" / "es_MX-claude-high.onnx"),
    )
)
PIPER_VOICE = None
PIPER_LOCK = threading.Lock()
if PIPER_MODEL.is_file():
    try:
        from piper import PiperVoice

        PIPER_VOICE = PiperVoice.load(str(PIPER_MODEL))
        print(f"piper voice loaded from {PIPER_MODEL}")
    except Exception as error:
        print(f"piper voice failed to load: {error}", file=sys.stderr)


def decode_audio(data: bytes) -> np.ndarray:
    container = av.open(io.BytesIO(data))
    stream = container.streams.audio[0]
    resampler = av.AudioResampler(format="fltp", layout="mono", rate=16_000)
    chunks: list[np.ndarray] = []
    for frame in container.decode(stream):
        for resampled in resampler.resample(frame):
            chunks.append(resampled.to_ndarray().reshape(-1))
    for resampled in resampler.resample(None):
        chunks.append(resampled.to_ndarray().reshape(-1))
    container.close()
    if not chunks:
        return np.zeros(0, dtype=np.float32)
    return np.concatenate(chunks).astype(np.float32)


def synthesize(text: str) -> bytes:
    """Render text with the bundled Piper female voice and return a WAV."""
    if not PIPER_EXE.is_file():
        raise RuntimeError(f"piper executable not found: {PIPER_EXE}")
    if not PIPER_MODEL.is_file():
        raise RuntimeError(f"piper model not found: {PIPER_MODEL}")
    if PIPER_VOICE is not None:
        buffer = io.BytesIO()
        with PIPER_LOCK:
            with wave.open(buffer, "wb") as wav:
                PIPER_VOICE.synthesize_wav(text, wav)
        return buffer.getvalue()

    with tempfile.TemporaryDirectory(prefix="hs-tts-") as directory:
        output = Path(directory) / "reply.wav"
        result = subprocess.run(
            [str(PIPER_EXE), "-m", str(PIPER_MODEL), "-f", str(output)],
            input=text,
            text=True,
            capture_output=True,
            timeout=120,
        )
        if result.returncode != 0 or not output.is_file():
            detail = (result.stderr or result.stdout).strip()[:300]
            raise RuntimeError(f"piper failed: {detail}")
        return output.read_bytes()


def transcribe_fast(audio: np.ndarray) -> str:
    """Transcribe with Vosk's general Spanish model (low latency)."""
    if VOSK_MODEL is None:
        raise RuntimeError("vosk model is not available")
    pcm = (np.clip(audio, -1.0, 1.0) * 32767).astype(np.int16).tobytes()
    recognizer = KaldiRecognizer(VOSK_MODEL, 16_000)
    parts: list[str] = []
    for offset in range(0, len(pcm), 32_000):
        if recognizer.AcceptWaveform(pcm[offset : offset + 32_000]):
            parts.append(str(json.loads(recognizer.Result()).get("text", "")))
    parts.append(str(json.loads(recognizer.FinalResult()).get("text", "")))
    return " ".join(part for part in parts if part).strip()


def detect_wake(audio: np.ndarray) -> dict:
    """Run a Vosk grammar recognizer and report whether the wake phrase matched."""
    if VOSK_MODEL is None:
        raise RuntimeError("vosk wake model is not available")
    pcm = (np.clip(audio, -1.0, 1.0) * 32767).astype(np.int16).tobytes()
    recognizer = KaldiRecognizer(VOSK_MODEL, 16_000, WAKE_GRAMMAR)
    recognizer.SetWords(True)
    words: list[dict] = []
    chunks: list[str] = []
    for offset in range(0, len(pcm), 32_000):
        if recognizer.AcceptWaveform(pcm[offset : offset + 32_000]):
            payload = json.loads(recognizer.Result())
            chunks.append(str(payload.get("text", "")))
            words.extend(payload.get("result", []))
    final = json.loads(recognizer.FinalResult())
    chunks.append(str(final.get("text", "")))
    words.extend(final.get("result", []))
    text = " ".join(part for part in chunks if part).strip()
    confident = [
        word
        for word in words
        if float(word.get("conf", 0.0)) >= 0.45
        and str(word.get("word", "")).lower() in {"hola", "aura"}
    ]
    confident_words = {str(word.get("word", "")).lower() for word in confident}
    wake = "hola" in confident_words and "aura" in confident_words
    return {"wake": wake, "text": text, "words": words}


class Handler(BaseHTTPRequestHandler):
    server_version = "HomeServerSTT/0.1"

    def _cors(self) -> None:
        origin = self.headers.get("Origin", "")
        allow = origin if origin in ALLOWED_ORIGINS else "http://127.0.0.1:3000"
        self.send_header("Access-Control-Allow-Origin", allow)
        self.send_header("Access-Control-Allow-Methods", "POST, GET, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.send_header("Vary", "Origin")

    def _json(self, status: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self._cors()
        self.end_headers()
        self.wfile.write(body)

    def do_OPTIONS(self) -> None:  # noqa: N802 - http.server API
        self.send_response(204)
        self._cors()
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802 - http.server API
        if self.path == "/health":
            self._json(200, {"ok": True, "engine": "faster-whisper", "model": "base"})
        else:
            self._json(404, {"error": "not found"})

    def _speak(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0 or length > 64 * 1024:
            self._json(400, {"error": "invalid text length"})
            return
        try:
            payload = json.loads(self.rfile.read(length))
            text = str(payload.get("text", "")).strip()[:2_000]
        except Exception:
            self._json(400, {"error": "invalid JSON"})
            return
        if text == "":
            self._json(400, {"error": "empty text"})
            return
        try:
            wav = synthesize(text)
        except Exception as error:
            self._json(500, {"error": str(error)[:300]})
            return
        self.send_response(200)
        self.send_header("Content-Type", "audio/wav")
        self.send_header("Content-Length", str(len(wav)))
        self._cors()
        self.end_headers()
        self.wfile.write(wav)

    def _transcribe_fast(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0 or length > 20 * 1024 * 1024:
            self._json(400, {"error": "invalid audio length"})
            return
        data = self.rfile.read(length)
        try:
            audio = decode_audio(data)
            text = transcribe_fast(audio)
        except Exception as error:
            self._json(500, {"error": str(error)[:300]})
            return
        self._json(200, {"text": text, "duration": round(audio.size / 16_000, 2)})

    def _wake(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0 or length > 8 * 1024 * 1024:
            self._json(400, {"error": "invalid audio length"})
            return
        data = self.rfile.read(length)
        try:
            audio = decode_audio(data)
        except Exception as error:
            self._json(400, {"error": f"cannot decode audio: {error}"})
            return
        if audio.size < 4_000:
            self._json(200, {"wake": False, "text": ""})
            return
        try:
            result = detect_wake(audio)
        except Exception as error:
            self._json(500, {"error": str(error)[:300]})
            return
        self._json(200, result)

    def do_POST(self) -> None:  # noqa: N802 - http.server API
        if self.path == "/speak":
            self._speak()
            return
        if self.path == "/wake":
            self._wake()
            return
        if self.path == "/transcribe-fast":
            self._transcribe_fast()
            return
        if self.path != "/transcribe":
            self._json(404, {"error": "not found"})
            return
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0 or length > 25 * 1024 * 1024:
            self._json(400, {"error": "invalid audio length"})
            return
        data = self.rfile.read(length)
        try:
            audio = decode_audio(data)
        except Exception as error:
            self._json(400, {"error": f"cannot decode audio: {error}"})
            return
        if audio.size < 1_600:
            self._json(200, {"text": ""})
            return
        try:
            text = wakeword.transcribe(
                MODEL,
                audio,
                "es",
                None,
                beam_size=5,
                vad_filter=False,
                enhance=False,
            )
        except Exception as error:
            self._json(500, {"error": f"transcription failed: {error}"})
            return
        self._json(200, {"text": text, "duration": round(audio.size / 16_000, 2)})


def main() -> int:
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"stt server ready on http://{HOST}:{PORT} (base, es)")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())



