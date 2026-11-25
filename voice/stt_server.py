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

import asyncio
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse
from pathlib import Path

import av
import numpy as np
from vosk import KaldiRecognizer, Model

try:
    from wyoming.asr import Transcript
    from wyoming.audio import AudioChunk, AudioStart, AudioStop
    from wyoming.client import AsyncClient
    from wyoming.tts import Synthesize, SynthesizeStop
    from wyoming.wake import Detect, Detection, NotDetected

    WYOMING_AVAILABLE = True
except ImportError:  # local fallback still works without Wyoming
    WYOMING_AVAILABLE = False

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
import wakeword  # noqa: E402

HOST = "127.0.0.1"
PORT = 5055
WYOMING_ASR_URI = os.environ.get("WYOMING_ASR_URI", "tcp://127.0.0.1:10300")
WYOMING_TTS_URI = os.environ.get("WYOMING_TTS_URI", "tcp://127.0.0.1:10200")
WYOMING_WAKE_URI = os.environ.get("WYOMING_WAKE_URI", "tcp://127.0.0.1:10400")
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

DEFAULT_WAKE_WORD = "aurora"
WAKE_GRAMMAR = '["aurora", "[unk]"]'

PIPER_EXE = Path(
    os.environ.get("HS_PIPER_EXE", str(Path(sys.executable).with_name("piper.exe")))
)
PIPER_MODEL = Path(
    os.environ.get(
        "HS_PIPER_MODEL",
        str(ROOT.parent / ".demo" / "piper" / "es_ES-sharvard-medium.onnx"),
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




_EMOJI_RE = re.compile("[\U0001F000-\U0001FAFF\u2600-\u27BF\uFE0F\u200D\u20E3]+")


def sanitize_for_speech(text: str) -> str:
    """Remove emojis/markdown that a TTS engine would read aloud."""
    text = _EMOJI_RE.sub("", text)
    text = re.sub(r"[*_`#>|~]+", "", text)
    return re.sub(r"\s+", " ", text).strip()

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


def detect_wake(audio: np.ndarray, name: str | None = None) -> dict:
    """Run a Vosk grammar recognizer for the configured wake name."""
    if VOSK_MODEL is None:
        raise RuntimeError("vosk wake model is not available")
    phrase = (name or DEFAULT_WAKE_WORD).strip().lower()
    grammar = json.dumps([phrase, "[unk]"])
    pcm = (np.clip(audio, -1.0, 1.0) * 32767).astype(np.int16).tobytes()
    recognizer = KaldiRecognizer(VOSK_MODEL, 16_000, grammar)
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
    wanted = set(phrase.split())
    confident = [
        word
        for word in words
        if float(word.get("conf", 0.0)) >= 0.45
        and str(word.get("word", "")).lower() in wanted
    ]
    confident_words = {str(word.get("word", "")).lower() for word in confident}
    wake = bool(wanted) and wanted.issubset(confident_words)
    return {"wake": wake, "text": text, "words": words}


async def _wyoming_client(uri: str, read_timeout: float = 60.0) -> "AsyncClient":
    client = AsyncClient.from_uri(
        uri, connect_timeout=3.0, read_timeout=read_timeout
    )
    await client.connect()
    return client


def _pcm16(audio: np.ndarray) -> bytes:
    return (np.clip(audio, -1.0, 1.0) * 32767).astype(np.int16).tobytes()


async def wyoming_transcribe(audio: np.ndarray) -> str:
    """Transcribe through a Wyoming faster-whisper server."""
    if not WYOMING_AVAILABLE:
        raise RuntimeError("wyoming client is not installed")
    pcm = _pcm16(audio)
    client = await _wyoming_client(WYOMING_ASR_URI, read_timeout=120.0)
    try:
        await client.write_event(
            AudioStart(rate=16_000, width=2, channels=1).event()
        )
        step = 32_000
        for offset in range(0, len(pcm), step):
            await client.write_event(
                AudioChunk(
                    rate=16_000,
                    width=2,
                    channels=1,
                    audio=pcm[offset : offset + step],
                ).event()
            )
        await client.write_event(AudioStop().event())
        while True:
            event = await client.read_event()
            if event is None:
                return ""
            if Transcript.is_type(event.type):
                return Transcript.from_event(event).text.strip()
    finally:
        await client.disconnect()


async def wyoming_speak(text: str) -> bytes:
    """Render text through a Wyoming Piper server and return a WAV."""
    if not WYOMING_AVAILABLE:
        raise RuntimeError("wyoming client is not installed")
    client = await _wyoming_client(WYOMING_TTS_URI, read_timeout=120.0)
    try:
        await client.write_event(Synthesize(text=text).event())
        audio = bytearray()
        rate, width, channels = 22_050, 2, 1
        while True:
            event = await client.read_event()
            if event is None or SynthesizeStop.is_type(event.type):
                break
            if AudioStart.is_type(event.type):
                start = AudioStart.from_event(event)
                rate, width, channels = start.rate, start.width, start.channels
            elif AudioChunk.is_type(event.type):
                audio.extend(AudioChunk.from_event(event).audio)
            elif AudioStop.is_type(event.type):
                break
    finally:
        await client.disconnect()
    if not audio:
        raise RuntimeError("wyoming tts returned no audio")
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as handle:
        handle.setnchannels(channels)
        handle.setsampwidth(width)
        handle.setframerate(rate)
        handle.writeframes(bytes(audio))
    return buffer.getvalue()


async def wyoming_detect_wake(audio: np.ndarray, name: str | None = None) -> dict:
    """Detect a configurable wake word through a Wyoming Alkove server."""
    if not WYOMING_AVAILABLE:
        raise RuntimeError("wyoming client is not installed")
    phrase = (name or DEFAULT_WAKE_WORD).strip().lower()
    pcm = _pcm16(audio)
    # Alkove's default "after a pause" context needs a little lead-in silence.
    pcm = b"\x00\x00" * 11_200 + pcm
    client = await _wyoming_client(WYOMING_WAKE_URI, read_timeout=2.0)
    try:
        await client.write_event(Detect(names=[phrase]).event())
        await client.write_event(
            AudioStart(rate=16_000, width=2, channels=1).event()
        )
        step = 32_000
        for offset in range(0, len(pcm), step):
            await client.write_event(
                AudioChunk(
                    rate=16_000,
                    width=2,
                    channels=1,
                    audio=pcm[offset : offset + step],
                ).event()
            )
        await client.write_event(AudioStop().event())
        while True:
            try:
                event = await client.read_event()
            except Exception:
                return {"wake": False, "text": "", "engine": "wyoming-alkove"}
            if event is None:
                return {"wake": False, "text": "", "engine": "wyoming-alkove"}
            if Detection.is_type(event.type):
                detection = Detection.from_event(event)
                return {
                    "wake": True,
                    "text": detection.name or phrase,
                    "engine": "wyoming-alkove",
                }
            if NotDetected.is_type(event.type):
                return {"wake": False, "text": "", "engine": "wyoming-alkove"}
    finally:
        await client.disconnect()


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
            self._json(
                200,
                {
                    "ok": True,
                    "engine": "wyoming" if WYOMING_AVAILABLE else "faster-whisper",
                    "model": "base",
                    "wyoming": WYOMING_AVAILABLE,
                },
            )
        else:
            self._json(404, {"error": "not found"})

    def _speak(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0 or length > 64 * 1024:
            self._json(400, {"error": "invalid text length"})
            return
        try:
            payload = json.loads(self.rfile.read(length))
            text = sanitize_for_speech(str(payload.get("text", "")))[:2_000]
        except Exception:
            self._json(400, {"error": "invalid JSON"})
            return
        if text == "":
            self._json(400, {"error": "empty text"})
            return
        try:
            wav = asyncio.run(wyoming_speak(text))
        except Exception:
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
            try:
                text = asyncio.run(wyoming_transcribe(audio))
            except Exception:
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
        query = parse_qs(urlparse(self.path).query)
        name = (query.get("name") or [self.headers.get("X-Wake-Name", "")])[0]
        name = name.strip().lower() or None
        try:
            audio = decode_audio(data)
        except Exception as error:
            self._json(400, {"error": f"cannot decode audio: {error}"})
            return
        if audio.size < 4_000:
            self._json(200, {"wake": False, "text": ""})
            return
        try:
            try:
                result = asyncio.run(wyoming_detect_wake(audio, name))
            except Exception:
                result = detect_wake(audio, name)
        except Exception as error:
            self._json(500, {"error": str(error)[:300]})
            return
        self._json(200, result)

    def do_POST(self) -> None:  # noqa: N802 - http.server API
        path = urlparse(self.path).path
        if path == "/speak":
            self._speak()
            return
        if path == "/wake":
            self._wake()
            return
        if path == "/transcribe-fast":
            self._transcribe_fast()
            return
        if path != "/transcribe":
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
            try:
                text = asyncio.run(wyoming_transcribe(audio))
            except Exception:
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





