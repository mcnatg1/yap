# ADR 0009: Knowledge worker IPC protocol

**Date:** 2026-06-30
**Status:** Accepted (roadmap — Phase 7a)
**Builds on:** [ADR 0004](0004-background-diarization-okf-agents.md) (subprocess worker, FIFO, chunk manifest), [ADR 0006](0006-silero-agents-state-machine.md) (orchestrator owns queue depth)
**Amended by:** [ADR 0017](0017-knowledge-base-compiler.md) — in the **team profile**, the `yap-knowledge-worker` subprocess and the TCP JSON-lines IPC protocol defined here are **replaced by the `yap-server` KB compiler service** with REST/HTTP APIs. The chunk manifest schema (ADR 0004 §3) is preserved as the normalised document input format. The **solo/local-first profile** retains the TCP JSON-lines protocol on `YAP_KNOWLEDGE_PORT` as specified in this ADR.
**Amended by:** [ADR 0020](0020-meeting-capture-diarization-authority.md) - the vault, `SPEAKER_XX`, and diarization events below are historical. Current speaker evidence uses revisioned `Unknown` / session-scoped `Speaker N` results and is not coupled to the OKF worker protocol.

## Context

ADR 0004 mandates a **separate `yap-knowledge-worker` subprocess** and mentions "stdin/JSON lines, named pipe, or localhost socket" with `YAP_KNOWLEDGE_PORT` — but no concrete protocol or message types. This ADR pins the wire format so the worker and Tauri can be built independently.

## Decision

### Transport

**Localhost TCP socket**, JSON-lines (newline-delimited JSON, one message per line), UTF-8, localhost-only.

| Item | Decision |
|------|----------|
| Bind | `127.0.0.1:<YAP_KNOWLEDGE_PORT>` (default `8790`; Rust probes up on conflict) |
| Framing | One JSON object per `\n` |
| Direction | Full-duplex: host→worker requests, worker→host events |
| Why TCP over named pipe | Uniform across Windows/macOS/Linux; matches sidecar HTTP mental model; easy to test |

### Host → worker messages

```json
{ "type": "submit_chunk", "manifest": { ...ADR 0004 chunk manifest... } }
{ "type": "session_end",  "session_id": "uuid" }      // trigger stitch
{ "type": "health" }
{ "type": "shutdown" }
```

### Worker → host events

```json
{ "type": "ready",       "version": "..." }
{ "type": "queue_depth", "depth": 2 }                  // host enforces ≤3 / degraded
{ "type": "chunk_done",  "chunk_id": "...", "ms": 4200 }
{ "type": "chunk_failed","chunk_id": "...", "code": "ALIGN_FAILED", "quarantined": true }
{ "type": "session_stitched", "session_id": "...", "path": "conversations/<id>.md" }
{ "type": "log", "level": "info", "msg": "..." }       // also to knowledge-worker.log
```

### Queue & backpressure
- Worker processes **1 chunk at a time**; reports `queue_depth` after each enqueue/dequeue.
- **Host (orchestrator)** owns the cap: at depth ≥3 it sets `degraded:true` on new manifests ([ADR 0004 §10](0004-background-diarization-okf-agents.md), [ADR 0006](0006-silero-agents-state-machine.md)). Worker does not drop chunks.
- `session_end` flushes remaining + runs the Archivist stitch job.

### Error codes (worker)

| Code | Meaning | Worker action |
|------|---------|---------------|
| `ALIGN_FAILED` | aligner crashed/no timings | whole-chunk single speaker; continue |
| `DIAR_FAILED` | Historical speaker-runtime/cluster error | Preserve anonymous/unknown attribution; continue |
| `WRITE_FAILED` | OKF/markdown write error | move audio+JSON to `quarantine/`; continue |
| `BAD_MANIFEST` | missing required field | reject message; log; do not crash |

### Lifecycle
- Spawned by Tauri on first chunk; **idle exit after 5 min** empty queue + no active session ([ADR 0004 §10](0004-background-diarization-okf-agents.md)); Tauri restarts on next chunk.
- `BELOW_NORMAL` priority (Win) / `nice 10` (Unix); ORT 2 threads.
- Logs: `%LOCALAPPDATA%/Yap/logs/knowledge-worker.log` (chunk ms, queue depth, vault size).

## Consequences

### Positive
- Concrete, testable contract; worker and host decouple cleanly.
- Host-owned backpressure keeps the degraded-mode rule in one place.
- JSON-lines is trivial to log, replay, and unit-test.

### Negative
- TCP socket needs port management (shared probe logic with other sidecars).
- Versioning: protocol changes need a `version` handshake (in `ready`).

### Neutral
- Phase 7a; nothing emits manifests until live chunker (Phase 3 hook) + L3 land.

## Alternatives considered
- **Named pipe / stdin** — rejected: platform-specific quirks; harder to test than TCP.
- **HTTP/REST** — rejected: long-running jobs + server-push events fit a persistent socket better than request/response.
- **Shared SQLite queue** — deferred: viable but adds DB coupling; revisit if socket proves fragile.

## References
- [ADR 0004](0004-background-diarization-okf-agents.md) — manifest, FIFO, quarantine
- [ADR 0010](0010-okf-conversation-schema.md) — what the worker writes
