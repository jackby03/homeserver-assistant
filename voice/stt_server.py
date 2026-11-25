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
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import av
import numpy as np

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

    def do_POST(self) -> None:  # noqa: N802 - http.server API
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
                MODEL, audio, "es", "Hola Aura.", beam_size=3
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
