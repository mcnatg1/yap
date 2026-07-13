# ADR implementation status

**Status:** Living, non-normative implementation audit
**As of:** 2026-07-13; the Phase 3 local gate is complete. Exact GB10 private-link evidence remains pinned to `099e558a27a747a7a2f24ec4e86f9c13f7604c13`.
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
| [0001](adr/0001-dual-stt-backends.md) | Live fallback vs official batch principle retained; model details amended by 0014/0019 | Shared | **85** | In-process pinned Nemotron live fallback, warm lifecycle, capability-health connector state, and a durable imported server queue | No upload/WSS/drain or server inference | Phase 5 remote STT |
| [0002](adr/0002-crispasr-unified-stt-runtime.md) | Historical; replaced by 0019 | Client | **0** | No tracked or wired CrispASR runtime | Entire retired runtime path absent by design | Do not revive; use 0019 |
| [0003](adr/0003-long-term-voice-architecture.md) | Long-term principles retained; runtime and meeting details amended by 0014/0019/0020 | Shared | **55** | Capture, local ASR, overlay, history, hotkeys, and Windows injection cover early layers | No LID, official server batch, enrichment, OKF, agents, or KB | Follow canonical remaining phases 4–9, not its historical phase map |
| [0004](adr/0004-background-diarization-okf-agents.md) | Non-blocking background principles retained; diarization details superseded by 0020 | Shared | **45** | Independent bounded sinks, track/gap/chunk contracts, crash-safe recording, evidence/result types | No production speaker sink, worker, diarizer, alignment, OKF, or agents | Phase 8 uses 0020 authority; Phase 9 owns OKF/agents |
| [0005](adr/0005-llama-server-agents.md) | Accepted target; team placement amended by 0014 | Shared | **20** | Development-only Polish flow exists | Polish still calls Ollama; no bundled llama-server, manager, model pin, health gate, or profiles | Implement only when local/server LLM product work is activated |
| [0006](adr/0006-silero-agents-state-machine.md) | Accepted principles; routing amended by 0014/0019/0020 | Shared | **45** | Rust `RuntimeOrchestrator` skeleton, connector state/capability projection, and single warm Nemotron lifecycle | No Silero ONNX, production `vad_segments`, agent registry, LLM mutex/queue, upload, or server-processing transitions | Later preprocessing, remote transport, and LLM gates |
| [0007](adr/0007-forced-alignment-engine.md) | Accepted principle; exact engine requires revalidation | Shared | **10** | Aligned-word and revision contract placeholders exist | No aligner, overlap projection, benchmark, fixture, worker, or service | Phase 6 benchmark and implementation |
| [0008](adr/0008-speechbrain-lid-gate.md) | Accepted behavior; runtime/placement unresolved | Shared | **0** | Only generic validated language hints exist | No LID model/runtime, probes, confidence gate, cache, picker, or tests | Phase 6 client confirmation plus server/solo runtime decision |
| [0009](adr/0009-knowledge-worker-protocol.md) | Solo protocol retained historically; team transport superseded by 0017 | Shared | **10** | Capture/chunk prerequisites exist | No worker, socket protocol, lifecycle, backpressure events, quarantine, or stitcher | Team work follows 0017; solo worker remains deferred |
| [0010](adr/0010-okf-conversation-schema.md) | Accepted Markdown/raw-preservation principles; historical schema superseded by 0022 | Shared | **20** | Durable TXT transcripts and immutable JSON revisions exist | No Google OKF validator/writer, Yap profile, glossary/actions, migration, or compiler output | Implement the ADR 0022 profile in the Phase 9 compiler |
| [0011](adr/0011-vector-rag-retrieval.md) | Accepted retrieval principles; team projection amended by 0017/0022 | Server | **0** | None | No FTS/vector store, embeddings, chunker, RRF, calibration, citations, permission filtering, or graph projection | Phase 9 after authoritative OKF and access boundaries |
| [0012](adr/0012-mcp-server-surface.md) | Accepted Phase 9 target; team hosting amended by 0017 | Server | **0** | None | No MCP runtime, tools/resources, transport, opt-in, authorization, or tests | Phase 9 after permission-filtered KB APIs |
| [0013](adr/0013-global-hotkey-injection.md) | Accepted; Windows implementation active | Client | **160** | Dual shortcut commands, transactional registration, persistence, non-focusable overlay, target revalidation, Unicode injection, clipboard fallback, required native WDIO, and installer smoke evidence | Settings still use typed chord fields; paste-last has no default; no deliberate chord recorder/Cancel/Reset flow, macOS/Linux adapters, or broad real-app/elevation matrix | Replace the text fields with safe physical-chord capture and usable defaults, then expand native compatibility evidence |
| [0014](adr/0014-server-tier-compute-topology.md) | Canonical server topology, amended by 0016/0019/0020/0021 | Shared | **70** | Client route vocabulary/orchestrator projection, versioned OpenAPI/live contracts, bounded loopback capability-health API, tested server route-selection skeleton, validated connector health/capability/retry state, durable SQLite imported jobs, stable errors, safe binding, hardened host bootstrap, and an exact private-link GB10 transient health smoke | No persistent service, production workload router, queue drain, WSS/upload workers, TLS/QUIC edge, auth, model-pool runtime, or server inference | Phase 5 remote STT |
| [0015](adr/0015-two-pass-diarization-speaker-identity.md) | Superseded by 0020 | Server | **0** | No implementation of the retired ECAPA/VBx design | Entire retired design absent by intent | Do not implement; use 0020 |
| [0016](adr/0016-auth-identity-bridge.md) | Canonical Phase 7 decision | Shared | **15** | Evidence/result contracts require provenance for named server assertions | No MSAL, credential storage, Yap-token validation, identity DB, grants, enrollment, deletion, or audit | Phase 7 after the server boundary works privately |
| [0017](adr/0017-knowledge-base-compiler.md) | Canonical team KB/compiler decision; format/projection amended by 0022 | Server | **0** | None beyond repository documentation | No Lane 1 store, `yap-knowledge`, compiler, databases, permission inheritance, APIs, or IaC | Phase 9 after identity and result authority |
| [0018](adr/0018-three-repo-topology.md) | Accepted eventual topology; staged monorepo retained through MVP | Shared | **45** | `desktop/`, `server/`, `infra/`, and `docs/` staging layout exists | No three-repo split, independent CI/CD/access controls, link migration, or cross-repo version policy | Split only at Phase 10 after real server/knowledge boundaries |
| [0019](adr/0019-local-streaming-model-selection.md) | Canonical client fallback decision | Client | **175** | Pinned model revision/SHA artifacts, in-process sherpa Nemotron, warm lifecycle, setup controls, profiler, native tests, and local release/installer contract proof | No completed model-artifact license review, licensed speech/WER fixture, real-model CI accuracy smoke, or cross-platform evidence | Close licensing, fixture, accuracy, and cross-platform gates |
| [0020](adr/0020-meeting-capture-diarization-authority.md) | Canonical meeting/capture/identity authority | Shared | **90** | Source-aware sessions/tracks, exact gaps, bounded sinks, crash-safe recording/recovery, durable imported-job ledger, immutable evidence/result contracts, local/server naming restrictions | Production remains mic dictation; no loopback, meeting UX, speaker model, remote transport, server reconciliation, contacts, or identity service | Phase 5 transport, then Phase 8 speaker inference/reconciliation |
| [0021](adr/0021-http3-secure-edge-transport.md) | Accepted gated HTTP/3 edge target; Phase 3 remains loopback HTTP/1.1 | Shared | **0** | Transport-neutral HTTP/live contract principles and a TCP fallback direction are documented | No TLS/QUIC edge, UDP exposure, client HTTP/3 path, negotiated capability, authenticated live baseline, fallback drill, or transport benchmark | Complete the Phase 5 remote transport and Phase 7 authentication boundary, then benchmark and security-gate the HTTP/3 edge |
| [0022](adr/0022-google-okf-permission-safe-projections.md) | Canonical Phase 9 Google OKF and permission-safe projection boundary | Server | **0** | Decision, pinned upstream revision, enterprise profile, permission algebra, baseline/challenger boundary, generation protocol, and verification gates are documented | No Google OKF fixtures/validator, Yap profile compiler, `yap-knowledge`, Postgres relationship/permission ledger, pgvector baseline, virtual view, or Neo4j challenger benchmark | Implement after Phase 7 identity and Phase 8 result authority are available |

## Verification snapshot

The evidence below combines the completed full local Phase 3 matrix with scoped
post-gate reruns. After the final server-log and client queue corrections, the
affected server, frontend unit, production-build, and Playwright suites were
rerun. Rust/Clippy, native WDIO, release-contract, provenance, and installer
results are retained from the full matrix because those later commits did not
touch their code or contracts.

- Server contract, health service, and infra: **50/50 passed**. The post-GB10 request-target redaction fix also passed its focused API suite **23/23**.
- Frontend unit tests: **256/256 passed**; the production TypeScript/Vite build passed with 295 modules.
- Rust: **674 executed tests passed**; Clippy passed with warnings denied.
- Playwright: **19/19 passed** after the Phase 3 UI gate regressions were repaired in `b98feee`.
- Required native WDIO passed all four spec files and 10 required assertions; the optional hardware/model probe remained skipped.
- Release contract: **12/12 passed**.
- Safe installer evidence: the isolated `Yap.Test` NSIS bundle built successfully; local preserve-data and explicit token/sentinel delete smokes both passed with zero residual footprint and an unchanged installer SHA-256.
- Third-party provenance: the exact pinned Freeflow revision, license, and selected upstream file hashes verified.
- GB10 evidence is pinned to exact `099e558a27a747a7a2f24ec4e86f9c13f7604c13`: Ubuntu ARM64/Python 3.12 server, contract, and infra checks passed **49/49**; transient loopback health and the command-line production connector reached `Ready`; a separate refused-tunnel invocation reached `Retrying`; teardown left no Yap process or local/remote port-18765 listener.

Later local security and correctness fixes were not contained in the pinned GB10 artifact and do not inherit its live-node evidence. The GB10 run did not prove a persistent service or same-process native UI transition, and it introduced no upload, WSS, authentication, ASR, external listener, or firewall change.

These checks do not activate missing product gates. There is no committed licensed real-speech/WER fixture, meeting RTTM/diarization fixture suite, remote upload/drain or server inference path, or authenticated end-to-end test. Scores must be revised only when those authoritative artifacts change.
