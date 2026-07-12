# ADR implementation status

**Status:** Living, non-normative implementation audit
**As of:** 2026-07-12 at merged main `51931e5`
**Authority:** ADRs define decisions; current code and executable tests define implementation truth.

An ADR can be accepted while its implementation score is zero. Superseded ADRs remain in the table for historical completeness, but a low score on a superseded decision is not backlog authorization.

## Score scale

| Score | Meaning |
|------:|---------|
| **0** | Plan only, or intentionally unimplemented because a later ADR superseded it |
| **50** | Useful skeleton, contracts, or isolated prerequisites |
| **100** | Partial working path with major end-to-end gaps |
| **150** | Usable baseline with hardening, platform, fixture, or release gaps |
| **200** | Production/release ready across the ADR's stated scope |

Scores are evidence-based estimates, not percentages. The owner column uses **Client**, **Server**, or **Shared** for the canonical boundary; historical solo/team differences are called out in the row.

## Complete ADR audit

| ADR | Decision status / precedence | Owner | Score / 200 | Implemented evidence | Missing or blocking evidence | Next canonical gate or replacement |
|-----|------------------------------|-------|------------:|----------------------|------------------------------|------------------------------------|
| [0001](adr/0001-dual-stt-backends.md) | Live fallback vs official batch principle retained; model details amended by 0014/0019 | Shared | **85** | In-process pinned Nemotron live fallback, warm lifecycle, and an imported server-queue shell | No connector, upload/WSS, durable jobs, or server inference | Phase 3 contract/connector, then Phase 5 remote STT |
| [0002](adr/0002-crispasr-unified-stt-runtime.md) | Historical; replaced by 0019 | Client | **0** | No tracked or wired CrispASR runtime | Entire retired runtime path absent by design | Do not revive; use 0019 |
| [0003](adr/0003-long-term-voice-architecture.md) | Long-term principles retained; runtime and meeting details amended by 0014/0019/0020 | Shared | **55** | Capture, local ASR, overlay, history, hotkeys, and Windows injection cover early layers | No LID, official server batch, enrichment, OKF, agents, or KB | Follow canonical phases 3–9, not its historical phase map |
| [0004](adr/0004-background-diarization-okf-agents.md) | Non-blocking background principles retained; diarization details superseded by 0020 | Shared | **45** | Independent bounded sinks, track/gap/chunk contracts, crash-safe recording, evidence/result types | No production speaker sink, worker, diarizer, alignment, OKF, or agents | Phase 8 uses 0020 authority; Phase 9 owns OKF/agents |
| [0005](adr/0005-llama-server-agents.md) | Accepted target; team placement amended by 0014 | Shared | **20** | Development-only Polish flow exists | Polish still calls Ollama; no bundled llama-server, manager, model pin, health gate, or profiles | Implement only when local/server LLM product work is activated |
| [0006](adr/0006-silero-agents-state-machine.md) | Accepted principles; routing amended by 0014/0019/0020 | Shared | **45** | Rust `RuntimeOrchestrator` skeleton and single warm Nemotron lifecycle | No Silero ONNX, production `vad_segments`, agent registry, LLM mutex/queue, or real connector transitions | Phase 3 connector; later preprocessing and LLM gates |
| [0007](adr/0007-forced-alignment-engine.md) | Accepted principle; exact engine requires revalidation | Shared | **10** | Aligned-word and revision contract placeholders exist | No aligner, overlap projection, benchmark, fixture, worker, or service | Phase 6 benchmark and implementation |
| [0008](adr/0008-speechbrain-lid-gate.md) | Accepted behavior; runtime/placement unresolved | Shared | **0** | Only generic validated language hints exist | No LID model/runtime, probes, confidence gate, cache, picker, or tests | Phase 6 client confirmation plus server/solo runtime decision |
| [0009](adr/0009-knowledge-worker-protocol.md) | Solo protocol retained historically; team transport superseded by 0017 | Shared | **10** | Capture/chunk prerequisites exist | No worker, socket protocol, lifecycle, backpressure events, quarantine, or stitcher | Team work follows 0017; solo worker remains deferred |
| [0010](adr/0010-okf-conversation-schema.md) | Accepted principles; schema details require 0020 amendment | Shared | **20** | Durable TXT transcripts and immutable JSON revisions exist | No OKF/YAML writer, parser, glossary/actions, migration, or compiler output | Amend schema, then implement in Phase 9 compiler |
| [0011](adr/0011-vector-rag-retrieval.md) | Accepted Phase 9 target | Server | **0** | None | No FTS/vector store, embeddings, chunker, RRF, calibration, citations, or permission filtering | Phase 9 after authoritative KB and access boundaries |
| [0012](adr/0012-mcp-server-surface.md) | Accepted Phase 9 target; team hosting amended by 0017 | Server | **0** | None | No MCP runtime, tools/resources, transport, opt-in, authorization, or tests | Phase 9 after permission-filtered KB APIs |
| [0013](adr/0013-global-hotkey-injection.md) | Accepted; Windows implementation active | Client | **160** | Dual shortcut commands, transactional registration, persistence, non-focusable overlay, target revalidation, Unicode injection, clipboard fallback, required native WDIO, and installer smoke evidence | Settings still use typed chord fields; paste-last has no default; no deliberate chord recorder/Cancel/Reset flow, macOS/Linux adapters, or broad real-app/elevation matrix | Replace the text fields with safe physical-chord capture and usable defaults, then expand native compatibility evidence |
| [0014](adr/0014-server-tier-compute-topology.md) | Canonical server topology, amended by 0016/0019/0020 | Shared | **45** | Client route vocabulary/orchestrator skeleton, server health value/router tests, and host bootstrap | No network API, capability health, connector, durable jobs, queues, workers, TLS, or auth | Canonical Phase 3 contract/connector, then Phase 5 remote STT |
| [0015](adr/0015-two-pass-diarization-speaker-identity.md) | Superseded by 0020 | Server | **0** | No implementation of the retired ECAPA/VBx design | Entire retired design absent by intent | Do not implement; use 0020 |
| [0016](adr/0016-auth-identity-bridge.md) | Canonical Phase 7 decision | Shared | **15** | Evidence/result contracts require provenance for named server assertions | No MSAL, credential storage, Yap-token validation, identity DB, grants, enrollment, deletion, or audit | Phase 7 after the server boundary works privately |
| [0017](adr/0017-knowledge-base-compiler.md) | Canonical team KB decision | Server | **0** | None beyond repository documentation | No Lane 1 store, `yap-knowledge`, compiler, databases, permission inheritance, APIs, or IaC | Phase 9 after identity and result authority |
| [0018](adr/0018-three-repo-topology.md) | Accepted eventual topology; staged monorepo retained through MVP | Shared | **45** | `desktop/`, `server/`, `infra/`, and `docs/` staging layout exists | No three-repo split, independent CI/CD/access controls, link migration, or cross-repo version policy | Split only at Phase 10 after real server/knowledge boundaries |
| [0019](adr/0019-local-streaming-model-selection.md) | Canonical client fallback decision | Client | **175** | Pinned model revision/SHA artifacts, in-process sherpa Nemotron, warm lifecycle, setup controls, profiler, native tests, and local release/installer contract proof | No completed model-artifact license review, licensed speech/WER fixture, real-model CI accuracy smoke, or cross-platform evidence | Close licensing, fixture, accuracy, and cross-platform gates |
| [0020](adr/0020-meeting-capture-diarization-authority.md) | Canonical meeting/capture/identity authority | Shared | **90** | Source-aware sessions/tracks, exact gaps, bounded sinks, crash-safe recording/recovery, immutable evidence/result contracts, local/server naming restrictions | Production remains mic dictation; no loopback, meeting UX, speaker model, transport, job ledger, server reconciliation, contacts, or identity service | Phase 3/5 transport foundation, then Phase 8 speaker inference/reconciliation |

## Verification snapshot

Current branch evidence at this audit:

- Frontend unit tests: **262/262 passed**.
- Rust: **548 library tests plus 15 integration/parity tests passed**; Clippy passed with warnings denied.
- Server skeleton: **4/4 tests passed**.
- Production frontend build: passed.
- Playwright: **17/17 passed** after correcting stale history/recovery authority fixtures.
- Required native WDIO: **9/9 passed**, including overlay close-to-tray, shared tray restore, and real tray quit.
- Release contract: **11/11 passed**, including dirty-tree, untracked-input, mutable-seal, process-liveness, and NSIS hook regressions.
- Safe installer evidence: the isolated `Yap.Test` NSIS bundle built successfully; local preserve-data and explicit token/sentinel delete smokes both passed with zero residual footprint and an unchanged installer SHA-256.
- Third-party provenance: exact pinned Freeflow revision, license, and selected upstream file hashes verified.

These checks do not activate missing product gates. There is no committed licensed real-speech/WER fixture, meeting RTTM/diarization fixture suite, server connector, server inference path, or authenticated end-to-end test. Scores must be revised when those authoritative artifacts change.
