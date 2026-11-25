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
import threading
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
CAPTURE_RATE = 48_000
DECIMATION = CAPTURE_RATE // SAMPLE_RATE
DEFAULT_CONFIG = Path("/var/lib/homecore/config.json")
DEFAULT_URL = "http://127.0.0.1:3000"
WAKE_MAX_SECONDS = 4.0
WAKE_SILENCE_SECONDS = 0.45
WAKE_IDLE_TIMEOUT = 5.0
QUERY_MAX_SECONDS = 9.0
SILENCE_SECONDS = 1.2
DEFAULT_THRESHOLD = 0.004
DEFAULT_ASSISTANT_NAME = "Aura"
OUTPUT_DEVICE: int | str | None = None

# Console output is UTF-8 even on Windows consoles with a legacy code page;
# agent replies may contain emoji that cp1252 cannot encode.
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except (AttributeError, OSError):
    pass


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


def downsample_to_target(audio: np.ndarray) -> np.ndarray:
    """48 kHz device capture -> 16 kHz for Silero VAD and Whisper."""
    usable = (audio.size // DECIMATION) * DECIMATION
    if usable == 0:
        return audio.astype(np.float32)
    return audio[:usable].reshape(-1, DECIMATION).mean(axis=1).astype(np.float32)


def sample_level(device: int, seconds: float = 0.5) -> float | None:
    try:
        audio = sd.rec(
            int(seconds * CAPTURE_RATE),
            samplerate=CAPTURE_RATE,
            channels=1,
            device=device,
            dtype="float32",
        )
        sd.wait()
        return rms(downsample_to_target(audio.flatten()))
    except Exception:  # unsupported host API/sample rate: skip this device
        return None


def measure_noise_floor(device: str | int | None) -> float | None:
    """Measure a short, quiet noise floor for the selected microphone."""
    levels: list[float] = []
    for _ in range(3):
        try:
            levels.append(rms(record_window(0.25, device)))
        except Exception as error:
            print(f"[wakeword] noise measurement failed: {error}", file=sys.stderr)
            return None
    return min(levels) if levels else None


def choose_output_device() -> int | str | None:
    """Use the G435 headset output (the only test output) when available."""
    try:
        for index, device in enumerate(sd.query_devices()):
            if device["max_output_channels"] <= 0:
                continue
            name = device["name"].lower()
            if "g435" in name or "headset earphone" in name:
                print(f"[wakeword] output device [{index}] {device['name']}")
                return index
        default = sd.default.device[1]
        print(f"[wakeword] no G435 output found; using default output [{default}]")
        return default
    except Exception as error:
        print(f"[wakeword] output selection failed: {error}", file=sys.stderr)
        return None


def stream_probe_level(device: int, attempts: int = 6) -> float | None:
    """Probe with the real 512-sample streaming blocks and return their mean.

    Host APIs are inconsistent on Windows: MME can return digital silence for
    short blocks while WASAPI/DirectSound stream fine (and vice versa). Scoring
    by the mean of several blocks avoids selecting a device from a single lucky
    spike.
    """
    levels: list[float] = []
    for _ in range(attempts):
        try:
            audio = sd.rec(
                4800,  # 100 ms at 48 kHz; reliable on every Windows host API
                samplerate=CAPTURE_RATE,
                channels=1,
                dtype="float32",
                device=device,
            )
            sd.wait()
            levels.append(rms(downsample_to_target(audio.flatten())))
        except Exception:
            return None
    return sum(levels) / len(levels) if levels else None


def choose_input_device(name_filter: str | None = None) -> tuple[int, float] | None:
    """Pick the strongest input device, optionally filtered by name (G435)."""
    best: tuple[float, int] | None = None
    wanted = (name_filter or "").lower()
    for index, device in input_devices():
        if wanted and wanted not in device["name"].lower():
            continue
        level = stream_probe_level(index)
        if level is None:
            continue
        print(
            f"[wakeword] input [{index:2d}] stream peak={level:.6f} "
            f"{device['name']}"
        )
        if best is None or level > best[0]:
            best = (level, index)
    if best is None:
        print(
            f"[wakeword] no input device matched '{name_filter or '*'}'",
            file=sys.stderr,
        )
        return None
    if best[0] < 0.0002:
        print(
            "[wakeword] WARNING: selected microphone is almost silent. "
            "Check the headset is on, unmuted and its mic boom is down.",
            file=sys.stderr,
        )
    print(f"[wakeword] auto-selected input device [{best[1]}]")
    return best[1], best[0]


def rms(block: np.ndarray) -> float:
    if block.size == 0:
        return 0.0
    return float(np.sqrt(np.mean(np.square(block, dtype=np.float64))))


def record_window(seconds: float, device: str | int | None) -> np.ndarray:
    frames = int(seconds * CAPTURE_RATE)
    audio = sd.rec(
        frames,
        samplerate=CAPTURE_RATE,
        channels=1,
        dtype="float32",
        device=device,
    )
    sd.wait()
    return downsample_to_target(audio.flatten())


def enhance_audio(audio: np.ndarray) -> np.ndarray:
    """High-pass/spectral-denoise + AGC for clearer Whisper input.

    Mirrors the intent of Discord/WebRTC noise suppression without native
    dependencies: estimate the noise spectrum from the quietest frames, subtract
    it (spectral subtraction), then normalize speech level.
    """
    if audio.size < 1024:
        return normalize_audio(audio)
    samples = audio.astype(np.float64)
    samples = samples - samples.mean()
    frame_size = 512
    hop = 256
    window = np.hanning(frame_size)
    pad = frame_size
    padded = np.concatenate([np.zeros(pad), samples, np.zeros(pad + frame_size)])
    frame_count = max(1, (len(padded) - frame_size) // hop + 1)
    frames = np.stack(
        [padded[index * hop : index * hop + frame_size] for index in range(frame_count)]
    )
    spectrum = np.fft.rfft(frames * window, axis=1)
    magnitude = np.abs(spectrum)
    energies = magnitude.mean(axis=1)
    quiet_count = max(1, len(energies) // 5)
    quiet = np.argsort(energies)[:quiet_count]
    noise = magnitude[quiet].mean(axis=0)
    cleaned = np.maximum(magnitude - 1.4 * noise, 0.08 * magnitude)
    phase = np.exp(1j * np.angle(spectrum))
    restored = np.fft.irfft(cleaned * phase, n=frame_size, axis=1) * window
    output = np.zeros(len(padded))
    weight = np.zeros(len(padded))
    for index in range(frame_count):
        start = index * hop
        output[start : start + frame_size] += restored[index]
        weight[start : start + frame_size] += window**2
    weight[weight < 1e-6] = 1.0
    output /= weight
    output = output[pad : pad + len(samples)]
    level = rms(output)
    if level > 0:
        gain = min(0.08 / level, 8.0)
        output = np.clip(output * gain, -1.0, 1.0)
    return output.astype(np.float32)


class SileroVad:
    """Streaming Silero VAD (ONNX) used to detect speech robustly."""

    def __init__(self, model_path: str) -> None:
        import onnxruntime as ort

        self.session = ort.InferenceSession(
            model_path, providers=["CPUExecutionProvider"]
        )
        self.reset()

    def reset(self) -> None:
        self.state = np.zeros((2, 1, 128), dtype=np.float32)
        self.sr = np.array(SAMPLE_RATE, dtype=np.int64)

    def probability(self, frame: np.ndarray) -> float:
        chunk = frame[:512].astype(np.float32)
        if chunk.size < 512:
            chunk = np.pad(chunk, (0, 512 - chunk.size))
        try:
            probability, state = self.session.run(
                None,
                {
                    "input": chunk.reshape(1, -1),
                    "state": self.state,
                    "sr": self.sr,
                },
            )
        except Exception as error:
            print(f"[wakeword] Silero VAD failed: {error}", file=sys.stderr)
            return 0.0
        self.state = state
        return float(probability.reshape(-1)[0])


def record_utterance(
    device: str | int | None,
    threshold: float,
    idle_timeout: float = WAKE_IDLE_TIMEOUT,
    vad: SileroVad | None = None,
) -> np.ndarray | None:
    """Stream frames until speech + trailing silence; None on idle timeout.

    With Silero VAD, speech is detected by a neural probability instead of a raw
    volume threshold, which is far more robust with quiet headset microphones.
    """
    chunk_samples = 1600  # 100 ms at 16 kHz (4800 captured at 48 kHz)
    chunk = chunk_samples / SAMPLE_RATE
    preroll_frames = 4  # 4 x 100 ms before speech onset
    silence = WAKE_SILENCE_SECONDS
    deadline = time.monotonic() + idle_timeout
    if vad is not None:
        vad.reset()
    pre: list[np.ndarray] = []
    utterance: list[np.ndarray] = []
    speech_started = False
    quiet = 0.0
    speech_time = 0.0
    peak = 0.0
    while True:
        block = sd.rec(
            4800,  # 100 ms at 48 kHz; splits into 3 Silero frames of 512
            samplerate=CAPTURE_RATE,
            channels=1,
            dtype="float32",
            device=device,
        )
        sd.wait()
        mono = downsample_to_target(block.flatten())
        level = rms(mono)
        peak = max(peak, level)
        probability = None
        if vad is not None:
            probabilities = [
                vad.probability(mono[offset : offset + 512])
                for offset in (0, 512, 1024)
            ]
            probability = max(probabilities)
        energy_gate = max(threshold * 1.5, 0.002)
        is_speech = (probability is not None and probability >= 0.5) or (
            level >= energy_gate
        )
        quiet_frame = (probability is None or probability < 0.35) and (
            level < energy_gate
        )
        if not speech_started:
            pre.append(mono)
            if len(pre) > preroll_frames:
                pre.pop(0)
            if is_speech:
                speech_started = True
                detail = (
                    f"vad={probability:.2f}, rms={level:.5f}"
                    if probability is not None
                    else f"rms={level:.5f}"
                )
                print(f"[wakeword] speech detected ({detail})")
                utterance = [*pre, mono]
                pre.clear()
                speech_time = chunk
                quiet = 0.0
            elif time.monotonic() >= deadline:
                print(
                    f"[wakeword] no speech (peak={peak:.5f}, "
                    f"threshold={threshold:.5f})"
                )
                return None
            continue
        utterance.append(mono)
        if quiet_frame:
            quiet += chunk
        else:
            quiet = 0.0
            speech_time += chunk
        if quiet >= silence and speech_time >= 0.3:
            return np.concatenate(utterance)
        if len(utterance) * chunk >= WAKE_MAX_SECONDS:
            return np.concatenate(utterance)


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
            int(chunk * CAPTURE_RATE),
            samplerate=CAPTURE_RATE,
            channels=1,
            dtype="float32",
            device=device,
        )
        sd.wait()
        mono = downsample_to_target(block.flatten())
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
    vad_filter: bool = True,
) -> str:
    """Transcribe a window locally; model/driver failures must not kill the loop."""
    try:
        audio = enhance_audio(audio)
        segments, _info = model.transcribe(
            audio,
            language=language if language in {"es", "en"} else None,
            beam_size=beam_size,
            vad_filter=vad_filter,
            condition_on_previous_text=False,
            initial_prompt=initial_prompt,
        )
        return " ".join(segment.text.strip() for segment in segments).strip()
    except Exception as error:  # local model/driver failure: keep listening
        print(f"[wakeword] transcription failed: {error}", file=sys.stderr)
        return ""


def play_wav(path: Path) -> bool:
    """Play a local 16-bit WAV through the Windows default output (headset)."""
    if os.name == "nt":
        try:
            import winsound

            winsound.PlaySound(str(path), winsound.SND_FILENAME)
            return True
        except Exception as error:
            print(f"[wakeword] winsound playback failed: {error}", file=sys.stderr)
    try:
        import wave

        with wave.open(str(path), "rb") as handle:
            channels = handle.getnchannels()
            width = handle.getsampwidth()
            rate = handle.getframerate()
            frames = handle.readframes(handle.getnframes())
        if width == 2:
            samples = np.frombuffer(frames, dtype=np.int16).astype(np.float32) / 32768.0
        elif width == 4:
            samples = np.frombuffer(frames, dtype=np.int32).astype(np.float32) / 2147483648.0
        else:
            print(f"[wakeword] unsupported WAV width {width}", file=sys.stderr)
            return False
        if channels > 1:
            samples = samples.reshape(-1, channels).mean(axis=1)
        sd.play(samples, rate, device=OUTPUT_DEVICE)
        sd.wait()
        return True
    except Exception as error:
        print(f"[wakeword] WAV playback failed: {error}", file=sys.stderr)
        return False


def piper_speak(executable: str, model: str, text: str) -> bool:
    """Synthesize with the local Piper neural voice and play the WAV."""
    if not executable or not model or not text:
        return False
    import subprocess as local_subprocess
    import tempfile

    try:
        tmp_root = os.environ.get("HS_TTS_TMP") or tempfile.gettempdir()
        with tempfile.TemporaryDirectory(
            prefix="hs-piper-", dir=tmp_root
        ) as directory:
            wav = Path(directory) / "speech.wav"
            result = local_subprocess.run(
                [executable, "-m", model, "-f", str(wav)],
                input=text,
                text=True,
                capture_output=True,
                timeout=120,
            )
            if result.returncode != 0:
                print(
                    f"[wakeword] piper failed: {result.stderr.strip()[:300]}",
                    file=sys.stderr,
                )
                return False
            return play_wav(wav)
    except Exception as error:
        print(f"[wakeword] piper error: {error}", file=sys.stderr)
        return False


def say(text: str, args: argparse.Namespace) -> bool:
    """Prefer the local neural voice, then an external TTS command."""
    if piper_speak(args.piper_exe, args.piper_model, text):
        return True
    return speak(args.tts_command, text)


def ask_agent(
    url: str,
    token: str,
    text: str,
    history: list[dict[str, str]],
    speak_reply: bool,
) -> str | None:
    """POST a turn to the local kiosk agent endpoint; None on failure."""
    payload = {"text": text, "speak": speak_reply, "history": history}
    try:
        response = post_json(f"{url}/api/v1/kiosk/agent/query", token, payload)
    except (urllib.error.URLError, TimeoutError, ValueError) as error:
        print(f"[wakeword] agent query failed: {error}", file=sys.stderr)
        return None
    return str(response.get("reply", "")).strip()


def listening_chime() -> None:
    """Soft two-note Siri-like earcon indicating the assistant is listening."""
    try:
        silence = np.zeros(int(0.03 * SAMPLE_RATE), dtype=np.float64)
        parts: list[np.ndarray] = []
        for frequency, duration in ((660.0, 0.14), (988.0, 0.18)):
            frames = int(duration * SAMPLE_RATE)
            time_axis = np.arange(frames, dtype=np.float64) / SAMPLE_RATE
            attack = np.clip(time_axis / 0.012, 0.0, 1.0)
            release = np.clip((duration - time_axis) / 0.04, 0.0, 1.0)
            envelope = np.minimum(attack, release)
            parts.append(
                0.50 * np.sin(2.0 * np.pi * frequency * time_axis) * envelope
            )
            parts.append(silence)
        audio = np.concatenate(parts[:-1])
        import tempfile
        import wave

        tmp_root = os.environ.get("HS_TTS_TMP") or tempfile.gettempdir()
        with tempfile.TemporaryDirectory(
            prefix="hs-chime-", dir=tmp_root
        ) as directory:
            path = Path(directory) / "chime.wav"
            pcm = (np.clip(audio, -1.0, 1.0) * 32767.0).astype(np.int16)
            with wave.open(str(path), "wb") as handle:
                handle.setnchannels(1)
                handle.setsampwidth(2)
                handle.setframerate(SAMPLE_RATE)
                handle.writeframes(pcm.tobytes())
            if play_wav(path):
                return
    except Exception as error:
        print(f"[wakeword] cue playback failed: {error}", file=sys.stderr)
    try:
        import winsound

        winsound.Beep(660, 140)
        winsound.Beep(988, 180)
    except Exception:
        pass


def speak(command: str | None, text: str) -> bool:
    """Pipe text to a local TTS command; returns False when it failed."""
    if not command or not text:
        return False
    import subprocess  # local only

    try:
        completed = subprocess.run(  # noqa: S603 - operator supplied local command
            command,
            input=text,
            text=True,
            shell=True,
            check=False,
            timeout=60,
        )
        return completed.returncode == 0
    except Exception as error:  # TTS must never kill the microphone loop
        print(f"[wakeword] TTS failed: {error}", file=sys.stderr)
        return False


def main() -> int:
    global OUTPUT_DEVICE
    parser = argparse.ArgumentParser(description="Local wake-word listener")
    parser.add_argument("--url", default=DEFAULT_URL, help="local kiosk base URL")
    parser.add_argument("--token", default="", help="kiosk token (default: read config)")
    parser.add_argument(
        "--config", type=Path, default=DEFAULT_CONFIG, help="daemon config.json"
    )
    parser.add_argument(
        "--silero-model",
        default="",
        help="Silero VAD ONNX model for robust neural speech detection",
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
        "--input-name",
        default=None,
        help="only consider input devices containing this name (e.g. G435)",
    )
    parser.add_argument(
        "--output-device",
        default=None,
        type=lambda value: int(value) if value.isdigit() else value,
        help="audio output device for chimes/TTS (default: G435 headset)",
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
        "--beam-size", type=int, default=3, help="Whisper beam size for requests"
    )
    parser.add_argument(
        "--wake-beam-size",
        type=int,
        default=1,
        help="Whisper beam size for the short wake phrase (lower = faster)",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=None,
        help="RMS silence threshold (default: measured from the microphone noise floor)",
    )
    parser.add_argument("--once", action="store_true", help="process a single window and exit")
    parser.add_argument(
        "--dump-audio",
        default="",
        help="write the last captured utterance to this WAV for diagnostics",
    )
    parser.add_argument(
        "--tts-command",
        default="",
        help="optional local TTS command reading text on stdin (e.g. piper --model ...)",
    )
    parser.add_argument(
        "--piper-exe",
        default="",
        help="Piper neural TTS executable (preferred over --tts-command)",
    )
    parser.add_argument(
        "--piper-model",
        default="",
        help="Piper .onnx voice model",
    )
    parser.add_argument(
        "--follow-up-seconds",
        type=float,
        default=7.0,
        help="seconds to wait for a follow-up turn without the wake word",
    )
    parser.add_argument(
        "--max-turns",
        type=int,
        default=8,
        help="maximum turns in one continuous conversation",
    )
    parser.add_argument(
        "--no-conversation",
        action="store_true",
        help="return to the wake word after a single answer",
    )
    args = parser.parse_args()

    if args.list_devices:
        print_input_devices()
        return 0

    device = args.device
    if device is None and args.input_name:
        choice = choose_input_device(args.input_name)
        if choice is not None:
            device, _level = choice
    if args.threshold is None:
        floor = measure_noise_floor(device)
        args.threshold = max(0.0013, min(floor * 4.0, 0.01)) if floor else DEFAULT_THRESHOLD
    if args.calibrate:
        level = rms(record_window(3.0, device))
        print(f"[wakeword] microphone RMS over 3 s: {level:.6f}")
        print(f"[wakeword] suggested threshold: {max(0.0008, level * 0.5):.6f}")
        return 0
    if args.auto_input and device is None:
        choice = choose_input_device()
        if choice is not None:
            device, _ambient = choice

    OUTPUT_DEVICE = (
        args.output_device
        if args.output_device is not None
        else choose_output_device()
    )

    vad = None
    if args.silero_model:
        try:
            vad = SileroVad(args.silero_model)
            print(f"[wakeword] Silero VAD loaded from {args.silero_model}")
        except Exception as error:
            print(
                f"[wakeword] Silero VAD unavailable: {error}",
                file=sys.stderr,
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

        window = record_utterance(device, args.threshold, vad=vad)
        if window is None:
            silent_windows += 1
            if silent_windows == 1 or silent_windows % 6 == 0:
                print(
                    "[wakeword] waiting for speech... (silence detected; if this "
                    "persists, power-cycle the G435 headset: it may be off, "
                    "asleep, hardware-muted or out of battery)"
                )
            if args.once:
                print("[wakeword] silence; no wake word detected")
                return 0
            continue
        silent_windows = 0
        if args.dump_audio:
            try:
                import wave as wave_writer

                with wave_writer.open(args.dump_audio, "wb") as handle:
                    handle.setnchannels(1)
                    handle.setsampwidth(2)
                    handle.setframerate(SAMPLE_RATE)
                    pcm = np.clip(window, -1.0, 1.0)
                    handle.writeframes(
                        (pcm * 32767).astype(np.int16).tobytes()
                    )
                print(
                    f"[wakeword] captured {len(window) / SAMPLE_RATE:.2f}s "
                    f"to {args.dump_audio}"
                )
            except Exception as error:
                print(f"[wakeword] dump failed: {error}", file=sys.stderr)
        started = time.monotonic()
        transcript = transcribe(
            model,
            window,
            args.language,
            initial_prompt,
            args.wake_beam_size,
            vad_filter=False,
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
            # Do not block with a 2 s TTS prompt: start capturing immediately.
            # The user naturally starts talking right after the wake word, so
            # only a short Siri-like chime is played before the request
            # recorder opens; the chime runs in a thread so capture starts at
            # once and the beginning of the request is never lost.
            listening_chime()
            print("[wakeword] listening for the request...")
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
        use_local_tts = bool(args.piper_exe and args.piper_model)
        reply = ask_agent(args.url, token, query, [], not use_local_tts)
        if reply is None:
            if args.once:
                return 1
            continue
        print(f"[wakeword] reply: {reply}")
        if use_local_tts:
            say(reply, args)
        if args.once:
            return 0

        # Continuous conversation: after the wake word the user can keep
        # talking without repeating "Oye Aura". History is sent each turn so
        # the agent keeps context.
        conversation: list[dict[str, str]] = [
            {"role": "user", "content": query},
            {"role": "assistant", "content": reply},
        ]
        if not args.no_conversation:
            for _turn in range(args.max_turns):
                listening_chime()  # cue: ready for a follow-up
                try:
                    follow_audio = record_until_silence(
                        args.follow_up_seconds,
                        SILENCE_SECONDS,
                        args.threshold,
                        device,
                    )
                except Exception as error:
                    print(
                        f"[wakeword] follow-up recording failed: {error}",
                        file=sys.stderr,
                    )
                    break
                follow = transcribe(
                    model, follow_audio, args.language, beam_size=args.beam_size
                )
                if not follow:
                    print("[wakeword] end of conversation (no follow-up)")
                    break
                print(f"[wakeword] follow-up: {follow}")
                if any(
                    phrase in follow.lower()
                    for phrase in ("adiós", "adios", "eso es todo", "hasta luego")
                ):
                    say("Hasta luego.", args)
                    break
                follow_reply = ask_agent(
                    args.url,
                    token,
                    follow,
                    conversation[-24:],
                    not use_local_tts,
                )
                if follow_reply is None:
                    break
                print(f"[wakeword] reply: {follow_reply}")
                if use_local_tts:
                    say(follow_reply, args)
                conversation.extend(
                    [
                        {"role": "user", "content": follow},
                        {"role": "assistant", "content": follow_reply},
                    ]
                )
                conversation = conversation[-24:]
        conversation.clear()


if __name__ == "__main__":
    raise SystemExit(main())


















