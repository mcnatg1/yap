# Spec: Phase 3 — Live English UX + audio thread

**Status:** Draft (2026-06-30)
**Implements:** [ADR 0001](../adr/0001-dual-stt-backends.md), [ADR 0002](../adr/0002-crispasr-unified-stt-runtime.md) (live endpoint), [ADR 0006](../adr/0006-silero-agents-state-machine.md) (Silero, orchestrator)
**Depends on:** [STT sidecar spec](phase-1-2-stt-sidecar.md) (live WS), [LLM sidecar spec](phase-a-d-llm-sidecar.md) (Scribe)
**Scope:** Ship **English-only live transcription** — mic capture, Silero VAD, Moonshine streaming, optional Scribe polish, in-app preview. No diarization/L3 (Phase 7), no global hotkey (Phase 7+).

---

## 1. Scope

### In scope
- Mic permission + device selection.
- Rust audio thread: capture → ring buffer → Silero VAD → frames to crispasr live WS.
- Live partial/final tokens rendered in an in-app panel (ghost preview).
- Optional Scribe polish on finals with 400 ms bypass + raw-mode indicator.
- “Save session” → WAV on disk (bridges to Phase 5 Cohere re-pass).
- Orchestrator states wired to UI.

### Out of scope
- Cross-app text injection, global hotkey ([ADR 0013](../adr/0013-global-hotkey-injection.md)).
- L3 enrichment / chunk manifests (Phase 7) — but the **chunker hook** is built (writes `vad_segments`), just not consumed.
- Multilingual live (future ADR).
- LID on live (Phase 4 is batch-only).

---

## 2. Audio thread architecture

```
cpal input stream (16 kHz mono f32)
  → lock-free ring buffer (audio callback only enqueues; never blocks / fsync)
  → VAD worker thread:
       Silero ONNX (ort) per 30 ms frame → speech prob
       state: silence / speech; debounce
  → on speech: stream PCM frames to crispasr live WS
  → on silence ≥ 1.5–2 s AND speech buffer ≥ 30 s: emit chunk-cut event
       (async .opus writer thread; manifest carries vad_segments) [hook only in P3]
```

### Decisions

| Choice | Decision | Why |
|--------|----------|-----|
| Capture crate | **`cpal`** | Cross-platform, low-level control, already common in Tauri audio |
| VAD runtime | **`ort` (onnxruntime) + bundled `silero_vad.onnx`** | Same ORT as worker; full control of thresholds; ~2 MB model |
| Not `silero-vad` crate | rejected default | Less control over framing/threshold; revisit only if `ort` integration churns |
| Threading | dedicated **audio callback** + **VAD/dispatch thread** + **writer thread** | Callback stays real-time; no model inference on the callback |
| Resampling | resample to 16 kHz mono before VAD/STT | Moonshine + Silero expect 16 kHz |

Silero is owned by **Rust on the audio path** ([ADR 0006](../adr/0006-silero-agents-state-machine.md)); the worker never re-runs it.

---

## 3. Orchestrator ↔ UI state map

`AppRuntimeState` ([ADR 0006](../adr/0006-silero-agents-state-machine.md)) → user-visible Live states:

| Runtime state | UI state | Indicator |
|---------------|----------|-----------|
| `Idle` | Idle | “Start live” button |
| `LiveReady` (moonshine loaded) | Ready | Mic armed, “Listening soon…” |
| `LiveActive` + silence | Listening | Animated mic / waveform idle |
| `LiveActive` + speech | Transcribing | Live waveform + streaming partials (ghost text, dimmed) |
| Scribe within budget | Polished | Final text settles (normal weight) |
| Scribe over 400 ms / skipped | **Raw mode** | Small “raw” badge on the segment; tooltip “Polished copy unavailable in time” |
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

- **Partials**: dimmed/italic, replaced in place as Moonshine revises.
- **Finals**: committed to normal weight; Scribe may replace a final with its polished version (dual-track stored: raw + polished).
- User can toggle **Raw / Polished** view of the whole session.
- “Save session” writes WAV (+ raw and polished text). In Phase 5 this WAV feeds a Cohere re-pass; in Phase 7 it feeds L3.

---

## 6. Error codes (live-specific, extends STT catalog)

| Code | Cause | UI |
|------|-------|----|
| `MIC_DENIED` | permission refused | Explainer + open settings |
| `MIC_UNAVAILABLE` | device busy/unplugged | “Microphone unavailable.” pick another |
| `LIVE_WS_DROPPED` | sidecar WS closed | Auto-reconnect once; else stop + raw of buffered |
| `STT_BACKEND_BUSY` | batch running | “Stop the current transcription to go live.” (orchestrator blocks dual STT) |
| `SCRIBE_TIMEOUT` | 400 ms exceeded | (silent) raw mode badge |

---

## 7. Acceptance criteria

- [ ] Speaking English shows partials < ~300 ms after speech onset (warm sidecar).
- [ ] Silence ≥ ~1.5 s finalizes the phrase.
- [ ] Scribe polishes finals when within 400 ms; otherwise raw with visible badge — never blocks the stream.
- [ ] Starting Live while batch runs is blocked with a clear message; switching unloads cohere → loads moonshine (orchestrator).
- [ ] “Save session” produces a playable WAV + raw/polished text.
- [ ] Mic-denied path is recoverable (no dead end).
- [ ] Chunk-cut hook writes a manifest with `vad_segments` (even though L3 doesn’t consume it yet).
- [ ] `prefers-reduced-motion` honored.

---

## 8. Open items

| Item | Resolve when |
|------|--------------|
| Exact Silero thresholds (speech prob, hangover ms) | Tune on real mics |
| Partial-render throttle (every N tokens vs time) | Measure UI cost |
| Whether Scribe runs per-final or per-utterance batch | After latency profiling |
| WAV vs Opus for saved sessions | Disk vs Phase 5 re-pass fidelity |
