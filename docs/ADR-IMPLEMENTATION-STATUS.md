# ADR implementation status

**Status:** Living, non-normative implementation audit
**As of:** 2026-07-14; the merged stock-NSIS Phase 3 evidence remains recorded below. The isolated Phase 4 private-ASR executable tree passed its one-time exact-head local/native/server/GB10 gate; reviewed PR and hosted closure remain pending.
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
| [0001](adr/0001-dual-stt-backends.md) | Live fallback vs official batch principle retained; model details amended by 0014/0019 | Shared | **85** | In-process pinned Nemotron live fallback, warm lifecycle, capability-health connector state, a durable imported server queue, and an isolated Cohere batch reference worker | No upload/WSS/drain or connected client/server inference | Phase 5 remote STT |
| [0002](adr/0002-crispasr-unified-stt-runtime.md) | Historical; replaced by 0019 | Client | **0** | No tracked or wired CrispASR runtime | Entire retired runtime path absent by design | Do not revive; use 0019 |
| [0003](adr/0003-long-term-voice-architecture.md) | Long-term principles retained; runtime and meeting details amended by 0013/0014/0019/0020 | Shared | **55** | Capture, local ASR, overlay, history, hotkeys, safe Windows clipboard delivery, and an isolated server batch reference cover early layers | No LID, connected/production server batch, enrichment, OKF, agents, or KB | Follow the canonical remaining phases, not its historical phase map |
| [0004](adr/0004-background-diarization-okf-agents.md) | Non-blocking background principles retained; diarization details superseded by 0020 | Shared | **45** | Independent bounded sinks, track/gap/chunk contracts, crash-safe recording, evidence/result types | No production speaker sink, worker, diarizer, alignment, OKF, or agents | Phase 8 uses 0020 authority; Phase 9 owns OKF/agents |
| [0005](adr/0005-llama-server-agents.md) | Accepted target; team placement amended by 0014 | Shared | **20** | Development-only Polish flow exists | Polish still calls Ollama; no bundled llama-server, manager, model pin, health gate, or profiles | Implement only when local/server LLM product work is activated |
| [0006](adr/0006-silero-agents-state-machine.md) | Accepted principles; routing amended by 0014/0019/0020 | Shared | **45** | Rust `RuntimeOrchestrator` skeleton, connector state/capability projection, and single warm Nemotron lifecycle | No Silero ONNX, production `vad_segments`, agent registry, LLM mutex/queue, upload, or server-processing transitions | Later preprocessing, remote transport, and LLM gates |
| [0007](adr/0007-forced-alignment-engine.md) | Accepted principle; exact engine requires revalidation | Shared | **10** | Aligned-word and revision contract placeholders exist | No aligner, overlap projection, benchmark, fixture, worker, or service | Phase 6 benchmark and implementation |
| [0008](adr/0008-speechbrain-lid-gate.md) | Accepted behavior; runtime/placement unresolved | Shared | **0** | Only generic validated language hints exist | No LID model/runtime, probes, confidence gate, cache, picker, or tests | Phase 6 client confirmation plus server/solo runtime decision |
| [0009](adr/0009-knowledge-worker-protocol.md) | Solo protocol retained historically; team transport superseded by 0017 | Shared | **10** | Capture/chunk prerequisites exist | No worker, socket protocol, lifecycle, backpressure events, quarantine, or stitcher | Team work follows 0017; solo worker remains deferred |
| [0010](adr/0010-okf-conversation-schema.md) | Accepted Markdown/raw-preservation principles; historical schema superseded by 0022 | Shared | **20** | Durable TXT transcripts and immutable JSON revisions exist | No Google OKF validator/writer, Yap profile, glossary/actions, migration, or compiler output | Implement the ADR 0022 profile in the Phase 9 compiler |
| [0011](adr/0011-vector-rag-retrieval.md) | Accepted retrieval principles; team projection amended by 0017/0022 | Server | **0** | None | No FTS/vector store, embeddings, chunker, RRF, calibration, citations, permission filtering, or graph projection | Phase 9 after authoritative OKF and access boundaries |
| [0012](adr/0012-mcp-server-surface.md) | Accepted Phase 9 target; team hosting amended by 0017 | Server | **0** | None | No MCP runtime, tools/resources, transport, opt-in, authorization, or tests | Phase 9 after permission-filtered KB APIs |
| [0013](adr/0013-global-hotkey-injection.md) | Accepted as amended 2026-07-14; Windows hotkey and safe-delivery implementation active | Client | **180** | Dual safe defaults, native-confirmed 15-second physical-chord enrollment with neutral/chord/release and modifier floors, normalization and reserved/conflict rejection, Cancel/per-action Reset, transactional registration rollback, one exact-bounds non-focusable island, clipboard-only delivery with Yap HWND ownership and visible paste guidance, focused native WDIO, stock installer contract, and a passing hosted disposable-Windows lifecycle | No macOS/Linux hotkey/clipboard adapters, broad real-app clipboard matrix, exact-field authority for safe direct insertion, or verified local real-model/hardware lifecycle on this machine | Expand native compatibility and hardware evidence without reintroducing synthesized input without exact-field authority |
| [0014](adr/0014-server-tier-compute-topology.md) | Canonical server topology, amended by 0016/0019/0020/0021/0023 | Shared | **100** | Client route vocabulary/orchestrator projection, versioned contracts, bounded loopback capability-health API, validated connector state, durable SQLite imported jobs, hardened host bootstrap, bounded in-memory owner-fair router, immutable Cohere/NGC/Python 3.12 lock, licensed WER fixture, and one isolated transient GB10 batch worker whose exact-head matrix and WER `0.0` gate passed | No persistent service, production/durable router, queue drain, WSS/upload workers, TLS/QUIC edge, auth, streaming pool, long-recording benchmark, or multi-worker capacity result | Complete reviewed Phase 4 closure, then Phase 5 remote STT |
| [0015](adr/0015-two-pass-diarization-speaker-identity.md) | Superseded by 0020 | Server | **0** | No implementation of the retired ECAPA/VBx design | Entire retired design absent by intent | Do not implement; use 0020 |
| [0016](adr/0016-auth-identity-bridge.md) | Canonical Phase 7 decision | Shared | **15** | Evidence/result contracts require provenance for named server assertions | No MSAL, credential storage, Yap-token validation, identity DB, grants, enrollment, deletion, or audit | Phase 7 after the server boundary works privately |
| [0017](adr/0017-knowledge-base-compiler.md) | Canonical team KB/compiler decision; format/projection amended by 0022 | Server | **0** | None beyond repository documentation | No Lane 1 store, `yap-knowledge`, compiler, databases, permission inheritance, APIs, or IaC | Phase 9 after identity and result authority |
| [0018](adr/0018-three-repo-topology.md) | Accepted eventual topology; staged monorepo retained through MVP | Shared | **45** | `desktop/`, `server/`, `infra/`, and `docs/` staging layout exists | No three-repo split, independent CI/CD/access controls, link migration, or cross-repo version policy | Split only at Phase 10 after real server/knowledge boundaries |
| [0019](adr/0019-local-streaming-model-selection.md) | Canonical client fallback decision | Client | **180** | Pinned model revision/SHA artifacts, in-process sherpa Nemotron, warm lifecycle, setup controls, profiler, native tests, local release/packaging contracts, and a passing hosted stock-NSIS lifecycle | No completed client-model artifact license review, local-Nemotron use of the licensed speech/WER fixture, real-model CI accuracy smoke, or cross-platform evidence | Close local-model licensing, fixture accuracy, and cross-platform gates |
| [0020](adr/0020-meeting-capture-diarization-authority.md) | Canonical meeting/capture/identity authority | Shared | **90** | Source-aware sessions/tracks, exact gaps, bounded sinks, crash-safe recording/recovery, durable imported-job ledger, immutable evidence/result contracts, local/server naming restrictions | Production remains mic dictation; no loopback, meeting UX, speaker model, remote transport, server reconciliation, contacts, or identity service | Phase 5 transport, then Phase 8 speaker inference/reconciliation |
| [0021](adr/0021-http3-secure-edge-transport.md) | Accepted gated HTTP/3 edge target; Phase 3 remains loopback HTTP/1.1 | Shared | **0** | Transport-neutral HTTP/live contract principles and a TCP fallback direction are documented | No TLS/QUIC edge, UDP exposure, client HTTP/3 path, negotiated capability, authenticated live baseline, fallback drill, or transport benchmark | Complete the Phase 5 remote transport and Phase 7 authentication boundary, then benchmark and security-gate the HTTP/3 edge |
| [0022](adr/0022-google-okf-permission-safe-projections.md) | Canonical Phase 9 Google OKF and permission-safe projection boundary | Server | **0** | Decision, pinned upstream revision, enterprise profile, permission algebra, baseline/challenger boundary, generation protocol, and verification gates are documented | No Google OKF fixtures/validator, Yap profile compiler, `yap-knowledge`, Postgres relationship/permission ledger, pgvector baseline, virtual view, or Neo4j challenger benchmark | Implement after Phase 7 identity and Phase 8 result authority are available |
| [0023](adr/0023-bounded-live-priority.md) | Accepted amendment to ADR 0014's priority rule | Server | **100** | The in-memory reference router prefers live work, forces one ready batch job after the configured live streak, preserves per-owner round robin and admission bounds, and has focused regression coverage | No durable queue, cancellation/recovery, authenticated owner, service integration, or measured production mixed-load tuning | Revalidate the bound with Phase 5 transport and production capacity evidence |

## Verification snapshot

The one-time Phase 3 local/native/server/GB10 implementation gate ran against
exact candidate `c3999b7b685dd668165d54b64d1af61e41adad05`. After the hosted lifecycle
exposed an early uninstall-cleanup assertion, implementation head
`a721121315c7a4bf5510212196141f17e9b237bd` added bounded convergence waiting
and passed hosted CI run `29293287930` plus stock NSIS lifecycle run
`29293291582`. This evidence-only documentation commit does not change
executable behavior and remains subject to the final checked-head PR gate.

- Server contract, health service, and infra: **50/50 passed** locally and **50/50 passed** from the immutable GB10 release.
- Frontend unit tests: **257/257 passed**; the production TypeScript/Vite build passed with 295 modules.
- Rust: **660/660 library tests** plus **27/27 integration tests** passed; format and all-target Clippy passed with warnings denied.
- Local live server boundary: **10/10 connector integration tests** passed against the bounded Python health process, followed by clean teardown.
- Playwright: **19/19 passed**.
- Required native WDIO passed all four spec files and 10 required assertions; the optional real-microphone/model probe remained explicitly skipped because no verified Nemotron model is installed.
- Release contract: **32/32 passed**. The exact pinned upstream revisions, license evidence, and selected source hashes verified, and the pnpm high-severity audit found no known vulnerabilities.
- Stock NSIS bundle: `Yap_0.1.0_x64-setup.exe`, 10,072,228 bytes, SHA-256 `c854a5b7b8e824fe305a9b78c7f0effc0b05c128125dddfb2163e0d730efb4b7`. It was built but not installed on the everyday Windows profile.
- Hosted closure: CI run `29293287930` passed frontend, server, native WDIO, Rust format/Clippy/tests/connector integration, the Windows advisory boundary, and the checksum-pinned RustSec audit on exact head `a721121315c7a4bf5510212196141f17e9b237bd`; CodeQL run `29293286157` passed for Actions, JavaScript/TypeScript, Python, and Rust.
- Installer closure: stock NSIS run `29293291582` passed on a disposable `windows-2025` runner. Its installer SHA-256 was `eeefad9860a5ca13c3ce240453d1877ba0f793391a9072f99d8ed16503d82655`; install/launch used `%APPDATA%\com.mcnatg1.yap`, silent uninstall converged, app data and the stock product registry record remained, and the install directory plus uninstall registry entry were removed.
- GB10 evidence: exact immutable release `c3999b7b685dd668165d54b64d1af61e41adad05`, archive SHA-256 `be7f43d757821c3e74d0ae2809599f5a84b369115d24afce42fe6687b1bf12e1`; ARM64/Python 3.12 checks passed **50/50**, tunneled production connector projected `Ready`, a separate refusal invocation projected `Retrying`, and teardown left no Yap process or local/remote port-18765 listener.

The GB10 run did not prove a persistent service or same-process native UI transition, and it introduced no upload, WSS, authentication, ASR, external listener, or firewall change. The local machine did not have `cargo-audit`; the checksum-pinned hosted RustSec lane supplied that evidence and passed on the implementation head.

### Phase 4 checked-head evidence

Exact executable candidate `309a2d427707e3483b2649f13940bd48dfaee836`
passed the one-time complete matrix. Frozen frontend install, the high-severity
pnpm audit, 32/32 release-contract tests, 261/261 Vitest tests, the 295-module
production build, and 23/23 Playwright tests passed. Python 3.12.13 passed
109/109 portable server tests. Rust format, warnings-denied all-target Clippy,
687/687 library tests, 27/27 integration tests, the no-`glib` Windows boundary,
and the checksum-pinned RustSec audit passed; the audit reported zero
vulnerabilities and 17 documented target-all warnings. The live connector
passed 10/10 with clean process/listener teardown. Native WDIO passed all four
spec files and 13 required assertions; its one optional real-microphone/model
probe remained explicitly skipped because no verified Nemotron model is
installed locally.

The disposable exact-head GB10 gate built ARM64 image
`sha256:8b98372d980b3d3ae3cb8bb5cc1498141d161d15157cbd6114339e7a31b8ddff`
and ran the locked Cohere revision on NVIDIA GB10 compute capability 12.1 in
CUDA/BF16. The returned runtime was Python 3.12.3, NVIDIA Torch
`2.13.0a0+8145d630e8.nv26.06`, and Torch CUDA 13.3; WER was `0.0` against the
`0.12` ceiling. Result SHA-256
`1a2850ad767489e00f6a496a46f95384d0d14b4a609d537a27a1304b80cfbbf0`
is bound by evidence SHA-256
`3157efc6845d3c03e05e22a5ad5d0a2e216de5ae26ae990501586a2dfa45312b`.
Before/after listener, firewall-policy, and service-unit observations matched;
the run opened no port or persistent service and left no Phase 4 container or
worker. Post-gate repository changes are evidence/status documentation only.

A private post-remediation security review is complete; its scan artifacts
remain local and are not repository or PR material. ADR 0014 remains at
**100/200** because the checked reference worker is still not a connected,
durable, authenticated, capacity-tested service.

These checks do not activate missing product gates. A committed licensed
real-speech/WER fixture and isolated server inference seam now exist, but there
is no meeting RTTM/diarization fixture suite, remote upload/drain, connected
server inference path, streaming pool, or authenticated end-to-end test.
Scores must be revised only when those authoritative artifacts change.
