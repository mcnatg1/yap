# Architecture Decision Records

This directory holds **Architecture Decision Records (ADRs)** for Yap — short, durable documents that capture significant technical choices, the context behind them, and their expected consequences.

## Numbering

ADRs are numbered sequentially with a four-digit prefix and a short slug:

```
NNNN-short-title-in-kebab-case.md
```

- **0001** is the first record in this repository.
- Numbers are never reused. If a decision is superseded, the original ADR stays in place; a new ADR references and replaces it.
- Gaps in numbering are acceptable if a draft was abandoned before merge.

## Format

Each ADR follows the [Nygard-style](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions) structure used across this repo:

| Section | Purpose |
|--------|---------|
| **Title / Date / Status** | What was decided and whether it is proposed, accepted, deprecated, or superseded |
| **Context** | Forces, constraints, and facts that made a decision necessary |
| **Decision** | The choice itself, stated plainly |
| **Consequences** | Expected outcomes — positive, negative, and neutral |
| **Implementation notes** | How the decision maps to code, rollout, and operational detail (when applicable) |
| **Alternatives considered** | Options that were evaluated and why they were not chosen |

Keep ADRs focused on one decision (or one tightly related cluster). Prefer updating product or design docs for UX copy; use ADRs when the choice has lasting architectural impact.

**Readable synthesis:** [VOICE-OS-ARCHITECTURE.md](../VOICE-OS-ARCHITECTURE.md) — layers, roadmap, two deployment profiles, hardening, viability assessment.

**Implementation audit:** [ADR-IMPLEMENTATION-STATUS.md](../ADR-IMPLEMENTATION-STATUS.md) — current client/server ownership, executable evidence, gaps, and 0–200 scores. Decision acceptance does not imply implementation completeness.

ADRs 0001–0013 cover the original **solo / local-first profile**. ADRs 0014–0018 introduce the **team / server profile**. ADR 0019 amends the local streaming model choice. ADR 0020 reconciles meeting capture, local anonymous speaker evidence, server-authoritative diarization, and identity privacy across both profiles. Later ADRs supersede conflicting details in earlier records.
ADR 0021 makes HTTP/3 the gated long-term client-facing transport target while preserving the bounded loopback service and TCP fallback.
ADR 0022 adopts pinned Google OKF v0.1 for Phase 9, requires a Postgres/pgvector plus typed-relationship baseline, and defines permission-safe projection gates for an optional Neo4j challenger without making any database the knowledge or authorization source-of-truth.

## Applicability and precedence

Use ADRs in this order:

1. A `Superseded` decision is historical and never authorizes implementation.
2. A later explicit `Amends` or `Supersedes` clause wins over an earlier conflicting detail.
3. ADRs 0014–0022 define the canonical client/server architecture and phase map. Earlier ADRs remain authoritative only for the principles or deployment profile their status names.
4. [VOICE-OS-ARCHITECTURE.md](../VOICE-OS-ARCHITECTURE.md) is the readable roadmap and status synthesis; it cannot silently override an ADR.
5. Build specs describe implementation. A `Draft` spec is not permission to ship a model, dependency, protocol, data-retention rule, or external surface absent an accepted ADR.

Every implementation plan must list its applied ADRs, superseded details it intentionally ignores, deferred decisions, exact acceptance tests, and phase boundary. Exact model/runtime names in a principle-only or historical ADR are benchmark candidates rather than defaults.

## Index

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-dual-stt-backends.md) | Dual STT backends: streaming live, server batch | Accepted principle; runtime/model amended by [0014](0014-server-tier-compute-topology.md) and [0019](0019-local-streaming-model-selection.md) |
| [0002](0002-crispasr-unified-stt-runtime.md) | CrispASR unified STT runtime (warm daemon + GGUF) | Historical runtime; active local path superseded by [0019](0019-local-streaming-model-selection.md) |
| [0003](0003-long-term-voice-architecture.md) | Long-term voice OS — recordings, SpeechBrain LID, roadmap | Accepted principles; phase map superseded by the canonical Voice OS roadmap |
| [0004](0004-background-diarization-okf-agents.md) | Background pipeline — diarization, micro-batches, OKF, agents | Accepted for non-diarization principles; diarization superseded by [0020](0020-meeting-capture-diarization-authority.md) |
| [0005](0005-llama-server-agents.md) | Bundled llama-server for LLM agents (CPU-first) | Accepted for solo/local; team execution amended by [0014](0014-server-tier-compute-topology.md) |
| [0006](0006-silero-agents-state-machine.md) | Silero VAD, agent profiles, runtime state machine | Accepted principles; active routing amended by [0014](0014-server-tier-compute-topology.md), [0019](0019-local-streaming-model-selection.md), and [0020](0020-meeting-capture-diarization-authority.md) |
| [0007](0007-forced-alignment-engine.md) | Forced-alignment engine for word→speaker | Accepted principle (canonical Phase 6); engine requires revalidation |
| [0008](0008-speechbrain-lid-gate.md) | SpeechBrain LID language gate | Accepted gate behavior (canonical Phase 6); model/runtime requires revalidation |
| [0009](0009-knowledge-worker-protocol.md) | Knowledge worker IPC protocol | Solo/local only; team protocol superseded by [0017](0017-knowledge-base-compiler.md) |
| [0010](0010-okf-conversation-schema.md) | OKF conversation schema | Accepted Markdown/YAML and raw-preservation principles; canonical Phase 9 format superseded by [0022](0022-google-okf-permission-safe-projections.md) |
| [0011](0011-vector-rag-retrieval.md) | Vector index + RAG retrieval (L6–L7) | Accepted principles; team storage/projection amended by [0017](0017-knowledge-base-compiler.md) and [0022](0022-google-okf-permission-safe-projections.md) |
| [0012](0012-mcp-server-surface.md) | MCP server surface | Accepted surface; team hosting amended by [0017](0017-knowledge-base-compiler.md) |
| [0013](0013-global-hotkey-injection.md) | Global hotkey + cross-app injection (L1) | Accepted (Windows active; cross-platform follow-on) |
| [0014](0014-server-tier-compute-topology.md) | Server-tier compute topology — thin client + GB-class workload router | Accepted (canonical Phases 3–5) |
| [0015](0015-two-pass-diarization-speaker-identity.md) | Two-pass diarization and speaker identity (ECAPA-TDNN + VBx) | Superseded by [0020](0020-meeting-capture-diarization-authority.md) |
| [0016](0016-auth-identity-bridge.md) | Authentication and voice identity bridge (Entra ID + MSAL) | Accepted (canonical Phase 7) |
| [0017](0017-knowledge-base-compiler.md) | Team knowledge base — source-of-truth, compiled disposable indexes, permission model | Accepted (canonical Phase 9; format and projection amended by [0022](0022-google-okf-permission-safe-projections.md)) |
| [0018](0018-three-repo-topology.md) | Three-repo topology (`yap-desktop` / `yap-server` / `yap-knowledge`) | Accepted (roadmap — canonical Phase 10) |
| [0019](0019-local-streaming-model-selection.md) | Local streaming model selection — Nemotron INT8 client fallback | Accepted (canonical Phase 2) |
| [0020](0020-meeting-capture-diarization-authority.md) | Meeting capture and diarization authority | Accepted (canonical Phase 8) |
| [0021](0021-http3-secure-edge-transport.md) | HTTP/3 transport evolution at the secure edge | Accepted (roadmap - gated after the Phase 5 remote transport and Phase 7 authentication baselines) |
| [0022](0022-google-okf-permission-safe-projections.md) | Google OKF and permission-safe knowledge projections | Accepted (canonical Phase 9 knowledge format and projection boundary) |

**Build specs** (how, not why): [docs/specs/](../specs/) — STT sidecar, LLM sidecar, live UX, testing.

**Readable synthesis (high/low pipeline charts + coverage matrix):** [VOICE-OS-ARCHITECTURE.md](../VOICE-OS-ARCHITECTURE.md)
