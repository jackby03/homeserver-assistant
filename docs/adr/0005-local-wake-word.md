# ADR 0005 — Local wake-word assistant with local speech recognition

- **Status:** accepted
- **Date:** 2026-11-25

## Context

The household wants an always-on assistant that answers when called by a name,
and the name must be customizable (`Aura`, `Casa`, `Jarvis`, ...). Browser speech
APIs (Chrome Web Speech) commonly send audio to a cloud service, which violates
the appliance's local-first and privacy requirements. Running a full ASR stack
inside the privileged Rust daemon would add native audio/model dependencies and
make the daemon harder to test.

## Decision

- The daemon owns the **wake-word decision state machine**: name matching,
  sensitivity, listening window, cooldown, kiosk events and permissions
  (`crates/hs-core/src/voice.rs`, `crates/hs-daemon/src/voice.rs`).
- Audio capture and speech-to-text run in an **optional unprivileged helper**
  (`voice/wakeword.py`) using a local model (faster-whisper). It sends only text
  transcripts to the loopback kiosk API.
- The wake name is a configurable field (`VoiceConfig.assistant_name`), exposed
  in the portal and mirrored by the kiosk hint. Changing it needs no restart and
  no model retraining: matching is transcript based (normalized tokens with a
  small typo tolerance), not a fixed vendor hotword model.
- Push-to-talk remains available; `activation_mode` selects wake word, button or
  both. Wake activation opens a bounded listening window (`listening_window_secs`)
  with a 1.5 s cooldown to avoid double triggers.
- The helper is optional: without it the kiosk still works with the "Hablar"
  button and the text/agent UI.

## Consequences

- Audio never leaves the appliance and transcripts are not persisted.
- The Rust daemon stays dependency-light and fully unit-testable (matcher and
  state machine have no audio dependencies).
- A local STT model is an optional deployment step; the appliance needs enough
  CPU for `tiny`/`base` Whisper models.
- The wake word is only as good as the local transcript; sensitivity is exposed
  and typo-tolerant matching keeps short names usable.

## Alternatives considered

- **Browser Web Speech API:** rejected (cloud processing, no offline guarantee).
- **Fixed vendor hotword models (Porcupine/openWakeWord):** rejected as the
  primary mechanism because custom names require model training; can be added
  later as an additional `WakeEngine` implementation.
- **ASR inside the daemon:** rejected (native deps, large models, harder testing).
