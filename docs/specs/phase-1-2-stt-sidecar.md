# Spec: Phase 1–2 — CrispASR STT sidecar integration

**Status:** Draft (2026-06-30)
**Implements:** [ADR 0001](../adr/0001-dual-stt-backends.md), [ADR 0002](../adr/0002-crispasr-unified-stt-runtime.md), [ADR 0006](../adr/0006-silero-agents-state-machine.md) (orchestrator hooks)
**Scope:** Replace the per-file Python batch path with a warm `crispasr` sidecar. **Batch (Cohere) only** ships in Phase 1–2; live (Moonshine) is wired but gated to Phase 3.

This is a **buildable contract** — IPC shapes, error codes, lifecycle, cutover, and acceptance tests. Architecture rationale lives in the ADRs; this doc says exactly what to build.

---

## 1. Scope

### In scope (Phase 1–2)

- Warm `crispasr` sidecar process, managed by Tauri (Rust).
- Batch file transcription via sidecar (Cohere GGUF), replacing `transcribe.py` on the happy path.
- `YAP_STT_BACKEND=crispasr|python` feature flag with automatic fallback.
- Model cache + first-run download/verify for the Cohere GGUF.
- Sidecar health surfaced in Setup UI.
- Structured error codes → toasts.

### Out of scope (later phases, referenced not built)

| Deferred | Phase | Note |
|----------|-------|------|
| Live Moonshine streaming | 3 | Sidecar exposes the endpoint; UI not wired |
| Silero VAD in Rust | 3 | Orchestrator owns it ([ADR 0006](../adr/0006-silero-agents-state-machine.md)) |
| SpeechBrain LID gate | 4 | Batch language stays manual picker |
| llama-server migration | A–D | Separate spec; Polish stays on Ollama until then |
| Knowledge worker / L3 | 7 | No chunk manifests emitted yet |

---

## 2. Current state (what exists today)

| Piece | File | Behavior |
|-------|------|----------|
| Batch command | `desktop/src-tauri/src/lib.rs` → `transcribe_files` | Spawns `.venv/Scripts/python.exe transcribe.py <paths...>`; one process per call; reads `.txt` paths from stdout lines |
| Runner | `transcribe.py` | Loads Cohere via torch/transformers, transcribes, writes sibling `.txt` |
| Setup probe | `lib.rs` → `setup_status` | Checks `python.exe` + `transcribe.py` exist |
| Polish | `desktop/src/polish.ts` | Calls Ollama `:11434` directly (unchanged this phase) |

**Cutover target:** `transcribe_files` calls the sidecar; `transcribe.py` becomes the `python` fallback backend.

---

## 3. Target process model

```
Yap (Tauri)
  ├─ crispasr sidecar      STT — one GGUF resident (cohere now; moonshine Phase 3)
  └─ (Ollama, external)    Polish — until llama-server spec ships
```

- **One** long-lived `crispasr` process per app session.
- Started **lazily** on first transcription (not at app launch) to keep idle RAM low; kept warm until idle timeout.
- Rust owns spawn / health / restart / shutdown. Frontend never spawns the sidecar.

---

## 4. IPC contract (Tauri Rust ↔ crispasr sidecar)

Transport: **HTTP on `127.0.0.1`**, localhost-only, no auth (single-user desktop). Port selection in §7.

> If the pinned CrispASR build does not expose an HTTP server, the Rust manager wraps the CLI (`crispasr --backend cohere -m … -f … -l …`) behind the **same internal Rust trait** so the rest of the app is transport-agnostic. HTTP is preferred; CLI-per-file is the documented fallback.

### 4.1 Health

```
GET /health
200 → { "status": "ok", "backend": "cohere" | "moonshine" | null, "model": "<file>", "version": "<crispasr-ver>" }
```

- `backend: null` = process up, no model loaded (Idle).
- Used by Setup status and orchestrator before mode switches.

### 4.2 Load / switch backend (exclusive residency)

```
POST /load   { "backend": "cohere" | "moonshine" }
200 → { "backend": "cohere", "model": "<file>", "loaded_ms": 1234 }
409 → { "code": "BUSY", "message": "transcription in progress" }
```

- Loading one backend **unloads** the other (hard invariant — moonshine XOR cohere, [ADR 0006](../adr/0006-silero-agents-state-machine.md)).
- Orchestrator calls this on `Idle → BatchReady` / `Idle → LiveReady`.

### 4.3 Batch transcription

```
POST /transcribe
{
  "audio_path": "C:/abs/path/recording.m4a",
  "language": "en",            // one of the 14 Cohere codes
  "quant": "q4_k" | "q8_0"     // optional; default q4_k
}
200 →
{
  "text": "full transcript ...",
  "language": "en",
  "duration_ms": 84210,        // audio length
  "decode_ms": 90120,          // wall time
  "backend": "cohere",
  "model": "cohere-transcribe-q4_k.gguf"
}
```

- **Writing the `.txt`** stays in Rust (keeps file-naming/history logic in one place), not the sidecar. Sidecar returns text; Rust writes sibling `.txt` exactly as today.
- Long files: single request for Phase 1–2 (no chunking). Progress streaming is §4.4.

### 4.4 Batch progress (optional, recommended)

For long files, stream progress so the UI shows movement instead of a spinner:

```
POST /transcribe?stream=1   → text/event-stream (SSE) or JSON lines:
{ "type": "progress", "pct": 0.42, "t_audio_ms": 35000 }
{ "type": "partial",  "text": "...so far..." }              // if backend supports it
{ "type": "final",    "text": "...", "decode_ms": 90120 }
{ "type": "error",    "code": "OOM", "message": "..." }
```

If the pinned build can’t stream, Rust falls back to the blocking call in §4.3 and the UI shows indeterminate progress. **Acceptance does not require streaming.**

### 4.5 Live streaming (Phase 3 — defined, not wired)

```
WS /live?backend=moonshine&lang=en   (English fixed)
client → PCM frames (16 kHz mono)
server → { "type": "partial", "text": "..." }
         { "type": "final",   "text": "...", "t0_ms": 0, "t1_ms": 1800 }
```

Documented here so the sidecar API is designed once. **No client work in Phase 1–2.**

---

## 5. Error code catalog

Sidecar returns structured codes; Rust maps to a `SttError` enum; frontend maps to toasts. **Single source of truth** — do not invent ad-hoc strings.

| Code | Cause | UI message | Recovery |
|------|-------|------------|----------|
| `MODEL_MISSING` | GGUF not in cache | “Transcription model not downloaded.” | Trigger download flow (§6) |
| `MODEL_CORRUPT` | Checksum/load fail | “Model file is damaged — re-downloading.” | Delete + re-download |
| `BAD_LANG` | Lang not in 14 codes | “That language isn’t supported for files yet.” | Reopen language picker |
| `OOM` | Allocation failure | “Not enough memory to transcribe this file.” | Suggest Q4 / close apps / shorter file |
| `AUDIO_DECODE` | Unreadable/corrupt media | “Couldn’t read this audio file.” | Skip file, continue queue |
| `SIDECAR_CRASH` | Process died mid-job | “Transcription engine restarted — retrying.” | Auto-restart + retry once |
| `SIDECAR_UNREACHABLE` | No response / port conflict | “Transcription engine isn’t responding.” | Restart; if repeated → offer `python` fallback |
| `BUSY` | Concurrent request | (internal) | Queue serializes; should not reach UI |
| `TIMEOUT` | Exceeded max decode budget | “This file took too long and was stopped.” | Offer retry / fallback |

**Rust enum (sketch):**

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum SttErrorCode {
    ModelMissing, ModelCorrupt, BadLang, Oom,
    AudioDecode, SidecarCrash, SidecarUnreachable, Busy, Timeout,
}
```

Exhaustive `match` on the code when mapping to messages (no catch-all that hides new variants).

---

## 6. Model cache & download

- Location: `YAP_MODELS_DIR` else `%LOCALAPPDATA%/Yap/models/` (Win) / `~/Library/Application Support/Yap/models/` (mac). Shared with future llama-server GGUF.
- Phase 1–2 model: `cohere-transcribe-q4_k.gguf` (~1.2 GB), pinned source [cstr/cohere-transcribe-03-2026-GGUF](https://huggingface.co/cstr/cohere-transcribe-03-2026-GGUF).
- **First-run flow:** if missing → download with progress UI → verify checksum → mark ready. Installer pre-cache is optional; on-first-launch download is the minimum bar.
- `crispasr-version.txt` (or `desktop/crispasr-version.txt`) pins the sidecar build; CI smoke-tests that exact pair.
- Optional `q8_0` lives beside `q4_k`; selected per request (§4.3) when the user enables “Higher quality batch”.

---

## 7. Sidecar lifecycle (Rust manager)

| Concern | Decision |
|---------|----------|
| **Spawn** | Lazy, on first `transcribe`/`load`; resolve binary from Tauri resources per OS/arch |
| **Port** | Default `127.0.0.1:8765`; if taken, probe up→`8775`; chosen port held in manager state, not hardcoded in frontend |
| **Ready gate** | Poll `/health` until `200` or 10 s timeout → else `SIDECAR_UNREACHABLE` |
| **Concurrency** | Manager serializes requests (one in flight); `BUSY` never surfaces to UI |
| **Crash detection** | Child exit watcher + request-level connection errors → `SIDECAR_CRASH` |
| **Restart policy** | Auto-restart once per job; reload last backend; retry the failed request a single time |
| **Idle shutdown** | Unload model after **10 min** idle (batch); kill process after a further idle window to free RAM |
| **App exit** | Kill child on Tauri shutdown; no orphan processes |
| **Logs** | `%LOCALAPPDATA%/Yap/logs/crispasr.log` (sidecar) + existing `local-transcribe.log` (Rust); never raw GGUF paths in primary UI |

Idle timeouts must match the orchestrator transitions in [ADR 0006](../adr/0006-silero-agents-state-machine.md) (BatchReady→Idle 10 m, LiveReady→Idle 5 m).

---

## 8. Migration & cutover

Feature flag **`YAP_STT_BACKEND`**:

| Value | Behavior |
|-------|----------|
| `crispasr` (default when healthy) | New sidecar path |
| `python` | Legacy `transcribe.py` path (current code, untouched) |
| unset | Try `crispasr`; if sidecar unhealthy at first use → fall back to `python` for the session and log it |

Rules:

1. `transcribe_files` becomes a thin dispatcher over a Rust `SttBackend` trait with two impls (`Crispasr`, `Python`).
2. The `python` impl is the **current code path verbatim** — keep it working as the safety net through Phase 2.
3. In-flight queue: if the sidecar dies mid-queue, finish remaining files via the active backend after one restart attempt; do not silently drop files.
4. Remove `python` default only after Phase 2 acceptance passes on all target OSes. `transcribe.py` stays in-tree as documented fallback.

---

## 9. Settings / UI surface

Per `PRODUCT.md` (technical setup is secondary):

- Setup status line: **“Transcription engine ready”** (green when `/health` ok + model cached) — not binary names or ports.
- Settings → **“Higher quality batch (Q8)”** toggle (default off) → sends `quant: "q8_0"`.
- Download progress: inline, not a modal, with size + cancel.
- Errors map to toasts via §5 (label + action, never a raw stack trace on the main screen).

---

## 10. Acceptance criteria

**Phase 1 — sidecar batch parity**

- [ ] Cold first run downloads + verifies Cohere GGUF with visible progress.
- [ ] Transcribing a known 60 s English clip via `crispasr` produces a `.txt` matching the `python` path within expected WER tolerance (spot-check, not byte-equal).
- [ ] Warm second file starts decoding with **no model reload** (verify via `loaded_ms`/logs).
- [ ] Setup shows “Transcription engine ready” only when `/health` is ok and model cached.
- [ ] Killing the sidecar mid-job surfaces `SIDECAR_CRASH`, auto-restarts, and completes the file.
- [ ] `YAP_STT_BACKEND=python` reproduces today’s behavior exactly.

**Phase 2 — hardening / default cutover**

- [ ] All §5 error codes reachable in a manual test matrix and mapped to toasts.
- [ ] Port-conflict path (occupy 8765) selects another port and still works.
- [ ] Idle timeout unloads model after 10 min; next file reloads cleanly.
- [ ] CI smoke test: start sidecar at pinned version, transcribe one fixture, assert non-empty text + exit clean — on each target OS.
- [ ] Queue of ≥5 files completes; one corrupt file yields `AUDIO_DECODE` and the queue continues.
- [ ] Default flips to `crispasr`; `python` remains as fallback.

---

## 11. Test fixtures

- `tests/fixtures/en-60s.wav` — clean English, known reference text.
- `tests/fixtures/multi-fr-30s.wav` — French, validates `-l fr`.
- `tests/fixtures/corrupt.m4a` — truncated, must yield `AUDIO_DECODE`.
- Golden transcripts stored beside fixtures; comparison is WER-tolerant, not exact.

---

## 12. Explicitly deferred (do not build now)

- Live WS client, Silero, chunk manifests, `vad_segments`.
- LID / language auto-detect (manual picker only).
- llama-server / Polish migration.
- Multi-window batch progress UI beyond a single progress bar.
- GPU offload (`-ngl > 0`).

---

## 13. Open items to resolve during build

| Item | Owner decision needed |
|------|------------------------|
| Does pinned CrispASR ship an HTTP server, or CLI-only? | Picks HTTP (§4) vs CLI-wrap fallback |
| SSE vs JSON-lines for progress | §4.4 transport |
| Exact decode timeout budget per minute of audio | §5 `TIMEOUT` |
| Checksum source (HF revision hash vs published sha256) | §6 verify step |
| Binary packaging per OS (resource path, signing) | Release pipeline |

These are integration unknowns, not architecture gaps — resolve with the pinned CrispASR build in hand.
