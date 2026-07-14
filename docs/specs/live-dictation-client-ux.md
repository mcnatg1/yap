# Spec: Live Dictation Client UX + Audio Thread

**Status:** Implemented baseline; native interaction, latency, and resilience hardening continue
**Implements:** [ADR 0006](../adr/0006-silero-agents-state-machine.md) (orchestrator/pre-warm), [ADR 0013](../adr/0013-global-hotkey-injection.md) (hotkeys/injection), [ADR 0019](../adr/0019-local-streaming-model-selection.md) (Nemotron local live fallback)
**Scope:** Define the **English-only client live path** as an implemented local baseline plus explicitly separate follow-on server/audio features.

> **2026-07-05 scope amendment:** The next live UI PR may introduce a top-positioned `live-overlay` surface, configurable capture hotkey, mic device settings, and typed live session state before cross-app injection. That bridge is specified in [Live Speaking Overlay And Controls](../superpowers/specs/2026-07-05-live-speaking-overlay-and-controls.md). Injection remains governed by [ADR 0013](../adr/0013-global-hotkey-injection.md).

> **2026-07-08 implemented local-fallback baseline:** The local live-transcription path uses one local model: Nemotron 3.5 ASR Streaming 0.6B INT8 through in-process `sherpa-onnx`. It keeps native punctuation, uses 1120 ms chunks until smaller chunks profile under real-time, and saves local live WAV/TXT output into Home history. Rust Silero ONNX, `vad_segments` chunk manifests, Opus/server WSS, Scribe, and diarization remain follow-on work.

> **2026-07-09 injection amendment:** Windows live completion captures and revalidates the stop-time external foreground/focused control, then inserts cleaned transcript text with Unicode `SendInput`. The overlay remains non-focusable, paste-last repeats only the dedicated last-completed transcript, and a visible clipboard fallback handles focus changes, held modifiers, or OS blocks. ADR 0013 owns this behavior.

> **2026-07-13 convergence implementation:** The client now reuses one `live-overlay` window and sends only semantic surfaces to Rust. Rust owns the production native bounds (`104×40` collapsed, `180×88` expanded, and exact compact status widths), anchors the top edge, and applies a rounded Windows region so transparent corner pixels are not interactive. Settings no longer accept typed chord strings: an explicitly armed physical-code recorder ignores repeats, rejects unsupported/bare/reserved chords, supports Cancel and per-action Reset, and relies on the existing transactional Rust registration rollback. Shipped defaults are `Ctrl+Shift+Space` for dictation and `Ctrl+Shift+Alt+V` for paste-last.

---

## 1. Scope

### Implemented baseline
- Mic permission + device selection.
- Top-positioned live overlay foundation and typed live session state.
- Transactional dictation and paste-last shortcut commands with safe defaults, explicit physical-chord recording, Cancel, and per-action Reset.
- CPAL capture → bounded channels → mono conversion/linear resampling → warm in-process sherpa Nemotron recognizer.
- 1120 ms local chunks with partial/final state updates.
- Windows stop-time target revalidation, Unicode insertion, and visible clipboard fallback.
- Local WAV/TXT save into Home history.
- Orchestrator states wired to UI.

### Follow-on client/server work
- Rust Silero VAD and reusable `vad_segments`.
- Opus chunk writer and server-ready manifests.
- Server live WSS connector, reconnect/backpressure policy, and local fallback handoff.
- Optional Scribe polish with a measured bypass budget.
- Server diarization and later enrichment phases.

### Out of scope
- macOS/Linux injection adapters and deeper accessibility/permission recovery ([ADR 0013](../adr/0013-global-hotkey-injection.md)).
- L3 enrichment/OKF and server diarization (canonical phases 8-9).
- Multilingual live (future ADR).
- LID on live (canonical Phase 6 remains batch-only).

---

## 2. Audio thread architecture

### Implemented baseline

```
cpal input callback
  → bounded raw-frame channel
  → channel downmix + linear resample to 16 kHz mono
  → bounded stream-sample channel
  → 1120 ms chunks into in-process Nemotron LiveStreamEngine
  → partial/final live state
  → stop-tail drain
  → target-validated Windows injection
  → WAV/TXT persistence
```

### Follow-on preprocessing/server path

```
cpal input stream (16 kHz mono f32)
  → lock-free ring buffer (audio callback only enqueues; never blocks / fsync)
  → VAD worker thread:
       Silero ONNX (ort) per 30 ms frame → speech prob
       state: silence / speech; debounce
  → on speech: stream PCM frames to the warm sherpa Nemotron recognizer
  → on silence ≥ 1.5–2 s AND speech buffer ≥ 30 s: emit chunk-cut event
       (async .opus writer thread; manifest carries vad_segments)
```

### Decisions

| Choice | Decision | Why |
|--------|----------|-----|
| Capture crate | **`cpal`** | Cross-platform, low-level control, already common in Tauri audio |
| Follow-on VAD runtime | **`ort` (onnxruntime) + bundled `silero_vad.onnx`** | Full control of thresholds; ~2 MB model |
| Not `silero-vad` crate | rejected default | Less control over framing/threshold; revisit only if `ort` integration churns |
| Threading | Current bounded capture + recognizer workers; follow-on dedicated **VAD/dispatch** and **writer** workers | Callback stays real-time; no model inference or file I/O on the callback |
| Resampling | resample to 16 kHz mono before VAD/STT | Nemotron + Silero expect 16 kHz |

When Silero lands, it is owned by **Rust on the audio path** ([ADR 0006](../adr/0006-silero-agents-state-machine.md)); the server/knowledge worker never re-runs it.

---

## 3. Orchestrator ↔ UI state map

`AppRuntimeState` ([ADR 0006](../adr/0006-silero-agents-state-machine.md)) → user-visible Live states:

| Runtime state | UI state | Indicator |
|---------------|----------|-----------|
| `Idle` | Idle | “Start live” button |
| `LiveReady` (Nemotron loaded) | Ready | Mic armed, “Listening soon…” |
| `LiveActive` + silence | Listening | Animated mic / waveform idle |
| `LiveActive` + speech | Transcribing | Live waveform + streaming partials (ghost text, dimmed) |
| Scribe within budget (follow-on) | Polished | Final text settles (normal weight) |
| Scribe over budget / skipped (follow-on) | **Raw mode** | Small “raw” badge on the segment; tooltip “Polished copy unavailable in time” |
| error | Error | Toast (§6) + return to Ready |
| saving | Saving | “Saving session…” → WAV path |

`prefers-reduced-motion`: waveform becomes a static level meter; partial→final crossfades instead of animating (per `PRODUCT.md` accessibility).

---

## 4. Mic permission flow

1. First “Start live”: request OS mic permission via Tauri.
2. Denied → inline explainer + button to OS settings; never a dead end.
3. Settings → **input device** picker (default system mic); remembered per app.
4. Pre-flight: 300 ms level check; if silent, warn “No input detected from <device>”.

---

## 5. Ghost preview behavior

- **Partials**: dimmed/italic, replaced in place as Nemotron revises.
- **Finals**: committed to normal weight and retained as the completed transcript used by automatic injection and paste-last.
- Follow-on Scribe may add a dual-track polished form without replacing the raw source.
- Current “Save session” writes WAV + raw text. Future server phases may reprocess the recording and Phase 8 diarization consumes server-ready manifests.

---

## 6. Error codes (live-specific, extends STT catalog)

| Code | Cause | UI |
|------|-------|----|
| `MIC_DENIED` | permission refused | Explainer + open settings |
| `MIC_UNAVAILABLE` | device busy/unplugged | “Microphone unavailable.” pick another |
| `LOCAL_STREAM_FAILED` | in-process recognizer/capture worker failed | Stop safely, save available audio/text, and expose retry |
| `LOCAL_RUNTIME_BUSY` | another client-local Nemotron session is active | Keep one local recognizer session; do not conflate with server jobs |
| `SERVER_WSS_DROPPED` (future) | server live stream disconnected | Retry/reconnect under the connector policy, then fall back locally when ready |
| `SERVER_BACKPRESSURE` (future) | server live pool is saturated | Keep interactive live prioritized; degrade/fallback without converting imported files to local jobs |
| `SCRIBE_TIMEOUT` (future) | Scribe budget exceeded | (silent) raw mode badge |

---

## 7. Acceptance criteria

### Implemented baseline

- [ ] First partial occurs within one 1120 ms audio chunk plus measured decode overhead; record p50/p95 on reference Windows hardware.
- [ ] At most one client-local Nemotron session runs. Server live streams and server batch jobs are independently scheduled by the server router and may coexist; imported files never become local fallback jobs.
- [ ] “Save session” produces a playable WAV + raw text and Home can recover canonical live sessions after restart.
- [ ] Stop-time Windows target remains foreground through final decoding before Unicode input; otherwise the full transcript is copied and manual-paste status is visible.
- [ ] Paste-last repeats only the dedicated last completed transcript and never an active partial.
- [ ] Mic-denied path is recoverable (no dead end).
- [ ] `prefers-reduced-motion` honored.

### Native interaction convergence

- [x] Collapsed and expanded native bounds match the visible island; hover expands the same tray-owned window downward without activating the app.
- [x] Only visible island pixels are interactive, with a 200 ms collapse grace period that preserves the pointer target.
- [x] Dictation and paste-last work immediately from documented defaults: `Ctrl+Shift+Space` and `Ctrl+Shift+Alt+V`.
- [x] Shortcut recording starts only after an explicit user action, stores only the final normalized chord, never logs raw events, and supports Cancel and per-action Reset.
- [x] Invalid, reserved, conflicting, or failed registrations leave the previous working shortcut active.

Focused evidence covers pure physical-code normalization and reserved-chord tests, transactional Rust registration tests, Playwright visual/state/reduced-motion tests, a 20-sample hover p95 at or below 220 ms, and native WDIO proof that one unfocused `live-overlay` changes from `104×40` to `180×88` and back while its webview root equals the visible island. The optional real-microphone/model lifecycle remains explicitly skipped when the verified Nemotron model is absent; that skip does not weaken the geometry or shortcut evidence.

### Follow-on preprocessing/server path

- [ ] Silence ≥ ~1.5 s finalizes a Silero speech segment.
- [ ] Scribe polishes finals when within its measured budget; otherwise raw with visible badge and never blocks the stream.
- [ ] Chunk-cut writes a server-ready manifest with `vad_segments`.

---

## 8. Open items

| Item | Resolve when |
|------|--------------|
| Exact Silero thresholds (speech prob, hangover ms) | Tune on real mics |
| Partial-render throttle (every N tokens vs time) | Measure UI cost |
| Whether Scribe runs per-final or per-utterance batch | After latency profiling |
| WAV vs Opus for saved sessions | Disk vs Phase 5 re-pass fidelity |
