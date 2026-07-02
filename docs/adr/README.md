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

ADRs 0001–0013 cover the **solo / local-first profile**. ADRs 0014–0018 introduce the **team / server profile** (DGX Spark server tier, two-pass diarization, auth, KB compiler, repo topology). Both profiles are normative; solo profile is the baseline for all users.

## Index

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-dual-stt-backends.md) | Dual STT backends: Moonshine live, Cohere batch | Accepted (implementation superseded by [0002](0002-crispasr-unified-stt-runtime.md)) |
| [0002](0002-crispasr-unified-stt-runtime.md) | CrispASR unified STT runtime (warm daemon + GGUF) | Accepted |
| [0003](0003-long-term-voice-architecture.md) | Long-term voice OS — recordings, SpeechBrain LID, roadmap | Accepted (roadmap) |
| [0004](0004-background-diarization-okf-agents.md) | Background pipeline — diarization, micro-batches, OKF, agents | Accepted (roadmap) |
| [0005](0005-llama-server-agents.md) | Bundled llama-server for LLM agents (CPU-first) | Accepted |
| [0006](0006-silero-agents-state-machine.md) | Silero VAD, agent profiles, runtime state machine | Accepted |
| [0007](0007-forced-alignment-engine.md) | Forced-alignment engine for word→speaker | Accepted (roadmap — 7a) |
| [0008](0008-speechbrain-lid-gate.md) | SpeechBrain LID language gate | Accepted (roadmap — 4) |
| [0009](0009-knowledge-worker-protocol.md) | Knowledge worker IPC protocol | Accepted (roadmap — 7a) |
| [0010](0010-okf-conversation-schema.md) | OKF conversation schema | Accepted (roadmap — 7c) |
| [0011](0011-vector-rag-retrieval.md) | Vector index + RAG retrieval (L6–L7) | Accepted (roadmap — 7e) |
| [0012](0012-mcp-server-surface.md) | MCP server surface | Accepted (roadmap — 7e) |
| [0013](0013-global-hotkey-injection.md) | Global hotkey + cross-app injection (L1) | Accepted (roadmap — 7+) |
| [0014](0014-server-tier-compute-topology.md) | Server-tier compute topology — thin client + DGX Spark workload router | Accepted (roadmap — Phase 8) |
| [0015](0015-two-pass-diarization-speaker-identity.md) | Two-pass diarization and speaker identity (ECAPA-TDNN + VBx) | Accepted (roadmap — Phase 10) |
| [0016](0016-auth-identity-bridge.md) | Authentication and voice identity bridge (Entra ID + MSAL) | Accepted (roadmap — Phase 9) |
| [0017](0017-knowledge-base-compiler.md) | Team knowledge base — source-of-truth, compiled disposable indexes, permission model | Accepted (roadmap — Phase 11) |
| [0018](0018-three-repo-topology.md) | Three-repo topology (`yap-desktop` / `yap-server` / `yap-knowledge`) | Accepted (roadmap — Phase 12) |

**Build specs** (how, not why): [docs/specs/](../specs/) — STT sidecar, LLM sidecar, live UX, testing.

**Readable synthesis (high/low pipeline charts + coverage matrix):** [VOICE-OS-ARCHITECTURE.md](../VOICE-OS-ARCHITECTURE.md)
