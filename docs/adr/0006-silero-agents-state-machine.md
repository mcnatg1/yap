# ADR 0006: Silero VAD, agent profiles, and runtime state machine

**Date:** 2026-06-30
**Status:** Accepted
**Builds on:** [ADR 0004](0004-background-diarization-okf-agents.md), [ADR 0005](0005-llama-server-agents.md)
**Amended by:** [ADR 0014](0014-server-tier-compute-topology.md) — in the **team profile**, the model-residency state machine (moonshine XOR cohere) moves to the **server-side workload router**; the client-side `RuntimeOrchestrator` becomes a **server-connector state machine** (Disconnected → Connecting → Connected → LiveStreaming / BatchUploading). Silero VAD remains client-side in both profiles (local chunk endpointing, `vad_segments`). The full state machine spec in §3 of this ADR is normative for the **solo/local-first profile**.

## Context

The Voice OS architecture references **Silero VAD** on the live path (silence detection, chunk boundaries, `vad_segments` for L3) and **eight agent personas** (Scribe through Coordinator). It also runs **three native sidecars/processes** on 16 GB CPUs.

Without explicit rules, we risk:

- Running Silero twice (L2 + L3) and wasting CPU
- Loading **Moonshine + Cohere + llama-server + worker** concurrently
- Multiple **background LLM jobs** (Student + Analyst + Curator) stacking on Scribe
- No clear **idle/evict** transitions → memory stays pinned

This ADR specifies **where Silero lives**, **scoped agent profiles**, and a **runtime state machine** so only one heavy STT model and bounded LLM/work load run at a time.

## Decision

### 1. Silero VAD — placement and reuse

| Use | Owner | Implementation |
|-----|--------|----------------|
| **Live mic — speech/silence** | **Tauri (Rust)** | Silero VAD **ONNX** (~2 MB) via `ort` or `silero-vad` crate on a **dedicated audio thread** |
| **Silence-anchored chunk cut** | Same Rust audio path | Trigger when silence ≥ **1.5–2 s** and speech buffer ≥ **30 s** |
| **`vad_segments` for L3 manifest** | Rust → chunk JSON | Millisecond intervals **relative to chunk**; required field |
| **Live STT gating** | Optional | Feed speech regions to crispasr stream; or rely on CrispASR `--vad` **only if** it does not duplicate Rust Silero (pick one path in implementation — **prefer single Silero in Rust** for chunker + segments) |
| **L3 diarization** | knowledge worker | **Must not** re-run Silero when `vad_segments` is non-empty |
| **CrispASR Firered VAD** | Optional fallback | Batch/long-file paths inside crispasr only if Rust Silero not run on that audio |

**Bundle:** ship `silero_vad.onnx` (or equivalent pinned Silero v4/v5 ONNX) under `%LOCALAPPDATA%/Yap/models/`.

**Not on hot path:** WeSpeaker, alignment, spectral clustering.

### 2. Agent profiles (scoped)

Each agent has a **profile**: trigger, layer, LLM use, timeout, priority class, and **mutex group** (only one active per group unless noted).

| Profile ID | Name | Layer | Trigger | Uses LLM | Timeout | Priority class | Mutex group |
|------------|------|-------|---------|----------|---------|----------------|-------------|
| `scribe` | Scribe | L2 | Raw STT phrase final | Yes (llama-server) | **400 ms** hard | **HOT** | `llm_hot` |
| `archivist` | Archivist | L3 | Chunk manifest complete | No | 30 s/chunk | BACKGROUND_IO | `worker_slot` |
| `student` | Student | L5 | New conversation file | Optional | 60 s | BACKGROUND_LLM | `llm_background` |
| `curator` | Curator | L5 | User answered Student | Yes | 120 s | BACKGROUND_LLM | `llm_background` |
| `auditor` | Auditor | L5 | Weekly timer / manual | Yes | 300 s | IDLE_ONLY | `llm_background` |
| `librarian` | Librarian | L7 | User KB query | No | 10 s | INTERACTIVE | `kb_read` |
| `analyst` | Analyst | L7 | Librarian pack ready | Yes | 60 s | INTERACTIVE | `llm_background` |
| `coordinator` | Coordinator | L7 | New conversation saved | Yes | 45 s | BACKGROUND_LLM | `llm_background` |

**Priority classes (enforce in Rust orchestrator):**

| Class | Max concurrent | Preempts | Notes |
|-------|----------------|----------|-------|
| **HOT** | **1** (Scribe only) | — | Exceed 400 ms → skip LLM, emit raw |
| **INTERACTIVE** | 1 LLM (`analyst`) + reads | Pauses BACKGROUND_LLM | User waiting |
| **BACKGROUND_LLM** | **1 queued** (FIFO) | None | Student, Curator, Analyst (offline), Coordinator, Auditor |
| **BACKGROUND_IO** | Worker **1 chunk at a time** | — | Archivist path inside worker |
| **IDLE_ONLY** | Auditor when app not LIVE/BATCH hot | — | Never run during `LiveActive` |

**Profile payloads (implementation):** store under `desktop/src/agents/profiles/` as JSON or Rust constants — system prompt template, max_tokens, temperature, failure fallback id. Do not hardcode eight prompts in one file.

**v1 shipped agents:** Scribe (Polish panel) only. Others **disabled** until phase flags enable them.

### 3. Runtime state machine (process + load residency)

Orchestrator lives in **Tauri Rust** (`RuntimeOrchestrator` or equivalent). Sidecars report health; orchestrator owns transitions.

#### States

```
AppRuntimeState:
  Idle                 # no STT model loaded; sidecars may be up empty
  BatchReady           # crispasr: cohere GGUF loaded
  BatchRunning         # transcribe queue active
  LiveReady            # crispasr: moonshine GGUF loaded
  LiveActive           # mic open, streaming
  BackgroundEnriching  # knowledge worker has ≥1 chunk (orthogonal badge)
  DegradedBackground   # FIFO overflow; stitch at session end
```

#### Transition rules (memory / CPU)

| From | Event | To | Side effects |
|------|-------|-----|--------------|
| Idle | User opens Transcribe / queue file | BatchReady | Load **cohere**; unload moonshine if loaded |
| BatchReady | Queue empty + idle timeout (10m) | Idle | Unload cohere |
| Idle | User opens Live | LiveReady | Load **moonshine**; unload cohere if loaded |
| LiveReady | Start mic | LiveActive | Pre-warm llama-server (Scribe) |
| LiveActive | Stop mic | LiveReady | Keep moonshine warm briefly |
| LiveReady | Idle timeout (5m) | Idle | Unload moonshine |
| LiveActive | User starts batch | BatchReady | **Stop mic first**; unload moonshine; load cohere |
| Any | Worker queue depth ≥3 | DegradedBackground | Set `degraded` on new chunks |
| DegradedBackground | Session end | BackgroundEnriching → Idle worker | Flush queue |

**Hard invariants:**

1. **At most one crispasr backend loaded:** `moonshine` XOR `cohere`.
2. **At most one HOT llama-server call** at a time (Scribe).
3. **At most one BACKGROUND_LLM job** queued; additional jobs coalesce or wait.
4. **knowledge-worker:** sequential chunk processing; **idle exit 5 min** after empty queue.
5. **Never** start Student/Curator/Auditor LLM during `LiveActive` (except Scribe HOT).
6. **Thread/process caps:** crispasr threads TBD per platform; llama-server `-t 4`; worker ORT **2** threads; worker **BELOW_NORMAL** priority.

#### ASCII state diagram

```
                    ┌─────────┐
         ┌─────────│  Idle   │─────────┐
         │         └────┬────┘         │
    open Live      open Batch      timeout
         │              │              │
         ▼              ▼              │
   ┌───────────┐ ┌────────────┐       │
   │ LiveReady │ │ BatchReady │       │
   └─────┬─────┘ └──────┬─────┘       │
    start mic      run queue          │
         │              │              │
         ▼              ▼              │
   ┌───────────┐ ┌────────────┐       │
   │LiveActive │ │BatchRunning│───────┘
   └───────────┘ └────────────┘
         │              │
         └────── XOR ────┘  (never both STT backends loaded)
```

### 4. Orchestrator API (Rust — sketch)

```rust
enum SttBackend { None, Moonshine, Cohere }

struct RuntimeOrchestrator {
  stt: SttBackend,
  app_state: AppRuntimeState,
  llm_hot_busy: bool,
  llm_background_queue: VecDeque<AgentJob>,
  worker_pending_chunks: u8, // max 3
}

impl RuntimeOrchestrator {
  fn request_stt(&mut self, backend: SttBackend) -> Result<(), Conflict>;
  fn try_run_scribe(&mut self, raw: &str) -> ScribeOutcome; // RawOnly | Polished
  fn enqueue_background_agent(&mut self, job: AgentJob); // rejects if LiveActive + wrong profile
  fn on_chunk_enqueued(&mut self) -> bool; // false → degraded
}
```

Frontend asks orchestrator before starting Live, queue, or Polish — not ad-hoc sidecar spawns.

## Consequences

### Positive

- Silero defined once on L2; L3 reuses segments — saves CPU.
- Agent scope prevents eight LLMs firing on a 16 GB box.
- State machine enforces **one STT model** and bounded queues — predictable RAM.
- v1 can ship with **Scribe profile only**; others gated by feature flags.

### Negative

- Rust orchestrator is **real engineering** — must be built and tested.
- Silero in Rust adds ONNX dep to Tauri (separate from crispasr/worker).
- Strict mutex may **delay** background agents — acceptable tradeoff.

## Alternatives considered

### Silero only inside knowledge worker

**Rejected.** Chunk boundaries and live path need VAD before L3; would re-run Silero and miss live chunk triggers.

### Silero only in CrispASR

**Rejected for chunker ownership.** Tauri still needs silence-anchored cuts and `vad_segments` without waiting on STT decode.

### No state machine — ad-hoc load/unload

**Rejected.** Causes dual residency and OOM on mode switch.

### LangGraph / heavy agent framework

**Rejected.** Desktop app uses Rust scheduler + llama-server HTTP; profiles as data, not a framework.

## References

- [VOICE-OS-ARCHITECTURE.md](../VOICE-OS-ARCHITECTURE.md)
- [ADR 0004](0004-background-diarization-okf-agents.md) — chunk manifest, worker subprocess
- [ADR 0005](0005-llama-server-agents.md) — llama-server Scribe
