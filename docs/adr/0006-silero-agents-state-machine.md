# ADR 0006: Silero VAD, agent profiles, and runtime state machine

**Date:** 2026-06-30
**Status:** Accepted
**Builds on:** [ADR 0004](0004-background-diarization-okf-agents.md), [ADR 0005](0005-llama-server-agents.md)
**Amended by:** [ADR 0014](0014-server-tier-compute-topology.md) and PR3 — model residency for Moonshine/Cohere moves to the **server-side workload router** in the team profile. The client-side `RuntimeOrchestrator` becomes a **server-connector state machine** (Disconnected → Connecting → Connected → LiveStreaming / BatchUploading) plus a local Moonshine v2 tiny fallback path. Silero VAD remains client-side in both profiles (local chunk endpointing, `vad_segments`). The old local `moonshine XOR cohere` state machine is historical context; PR3 does not load local Cohere.

## Context

The Voice OS architecture references **Silero VAD** on the live path (silence detection, chunk boundaries, `vad_segments` for L3) and **eight agent personas** (Scribe through Coordinator). It also runs **three native sidecars/processes** on 16 GB CPUs.

Without explicit rules, we risk:

- Running Silero twice (L2 + L3) and wasting CPU
- Loading **Moonshine fallback + server batch upload + llama-server + worker** without bounds
- Multiple **background LLM jobs** (Student + Analyst + Curator) stacking on Scribe
- No clear **idle/evict** transitions → memory stays pinned

This ADR specifies **where Silero lives**, **scoped agent profiles**, and a **runtime state machine** so local fallback, server upload, and bounded LLM/worker load do not compete blindly.

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
  FallbackReady        # crispasr: Moonshine v2 tiny fallback loaded
  FallbackRunning      # local degraded/offline transcription active
  ServerQueued         # batch file queued for GB-class server Cohere path
  ServerUploading      # server batch upload/job active
  LiveReady            # mic path ready
  LiveActive           # mic open, streaming
  BackgroundEnriching  # knowledge worker has ≥1 chunk (orthogonal badge)
  DegradedBackground   # FIFO overflow; stitch at session end
```

#### Transition rules (memory / CPU)

| From | Event | To | Side effects |
|------|-------|-----|--------------|
| Idle | User queues larger recording | ServerQueued | Prepare server job; do not load local Cohere |
| ServerQueued | Server available + user runs queue | ServerUploading | Upload/job through GB-class server Cohere path |
| ServerQueued | Server unavailable | Idle | Queue/block; do not silently degrade larger recordings |
| Idle | User opens Live or explicit offline fallback | FallbackReady | Load pinned **Moonshine v2 tiny** fallback |
| LiveReady | Start mic | LiveActive | Pre-warm llama-server (Scribe) |
| LiveActive | Stop mic | FallbackReady | Keep Moonshine fallback warm briefly |
| FallbackReady | Idle timeout (5m) | Idle | Unload Moonshine fallback |
| LiveActive | User starts batch | ServerQueued | **Stop mic first**; queue server job |
| Any | Worker queue depth ≥3 | DegradedBackground | Set `degraded` on new chunks |
| DegradedBackground | Session end | BackgroundEnriching → Idle worker | Flush queue |

**Hard invariants:**

1. **At most one local crispasr fallback loaded:** PR3 loads Moonshine v2 tiny only; no local Cohere backend.
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
 open fallback     queue batch     timeout
         │              │              │
         ▼              ▼              │
   ┌───────────────┐ ┌────────────┐   │
   │FallbackReady  │ │ServerQueued│   │
   └──────┬────────┘ └──────┬─────┘   │
    run fallback      upload/job      │
         │              │              │
         ▼              ▼              │
   ┌───────────────┐ ┌──────────────┐ │
   │FallbackRunning│ │ServerUploading│─┘
   └───────────────┘ └──────────────┘
         local Moonshine       server Cohere
```

### 4. Orchestrator API (Rust — sketch)

```rust
enum LocalSttBackend { None, MoonshineFallback }
enum BatchRoute { None, ServerCohere }

struct RuntimeOrchestrator {
  local_stt: LocalSttBackend,
  batch_route: BatchRoute,
  app_state: AppRuntimeState,
  llm_hot_busy: bool,
  llm_background_queue: VecDeque<AgentJob>,
  worker_pending_chunks: u8, // max 3
}

impl RuntimeOrchestrator {
  fn request_local_fallback(&mut self) -> Result<(), Conflict>;
  fn queue_server_batch(&mut self) -> Result<(), Conflict>;
  fn try_run_scribe(&mut self, raw: &str) -> ScribeOutcome; // RawOnly | Polished
  fn enqueue_background_agent(&mut self, job: AgentJob); // rejects if LiveActive + wrong profile
  fn on_chunk_enqueued(&mut self) -> bool; // false → degraded
}
```

Frontend asks orchestrator before starting Live, fallback, queue, or Polish — not ad-hoc sidecar spawns.

## Consequences

### Positive

- Silero defined once on L2; L3 reuses segments — saves CPU.
- Agent scope prevents eight LLMs firing on a 16 GB box.
- State machine enforces **one local fallback model** and bounded queues — predictable RAM.
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
