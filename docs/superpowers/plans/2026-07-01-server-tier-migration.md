# Server-Tier Migration — Architecture Plan

**Date:** 2026-07-01
**Author:** Architecture
**Status:** Superseded as an execution plan by the canonical roadmap in [VOICE-OS-ARCHITECTURE.md](../../VOICE-OS-ARCHITECTURE.md) and the meeting decision in [ADR 0020](../../adr/0020-meeting-capture-diarization-authority.md). Retained as historical rationale.

> **Knowledge-plan correction (2026-07-12):** [ADR 0022](../../adr/0022-google-okf-permission-safe-projections.md) supersedes this plan's claims that the original ADR 0010 frontmatter is unchanged and that the team profile should choose generically among Milvus/pgvector-class stores. Canonical Phase 9 now uses pinned Google OKF v0.1 plus the Yap Enterprise OKF profile, Postgres typed relationships, and a pgvector baseline. Neo4j is a benchmark-gated challenger. Do not implement the historical rows below as current storage or schema authority.

---

## Goal

Pivot Yap from a fully local-first stack to a **two-profile architecture**:

- **Solo / local-first profile** — today's on-device stack unchanged (offline, privacy-max, single user).
- **Team / server profile** — thin Tauri client + on-prem **GB-class server node** (GPU model pools, workload router, knowledge compiler, auth).

This plan enumerates every documentation change required: new ADRs, ADR amendments, phase-roadmap additions, and top-level doc updates.

---

## Context: what triggered this pivot

| Benchmark | Result | Implication |
|-----------|--------|-------------|
| Cohere batch (45-min file, CPU) | ~1 576 s (~26 min) | Not viable for real-time team use |
| Moonshine medium, batch use | ~0.55× realtime | Slower-than-realtime; not a batch win |
| GB-class GPU pool | Multiple concurrent workers | Parallelises across users; frees client CPU |

On-prem GPU is **not cloud** — it is org-owned hardware on an org-controlled LAN/VPN. This resolves the "local-first / no cloud STT" tension: "our hardware, our network" framing is accurate.

---

## New ADRs to create

| Proposed # | Filename | Purpose |
|------------|----------|---------|
| **0014** | `0014-server-tier-compute-topology.md` | Establish the server tier on GB-class hardware: thin client shell, workload router, model pools (streaming ASR / Cohere batch / LLM), two deployment profiles (solo local vs team server), on-prem-not-cloud framing, live path tradeoffs |
| **0015** | `0015-two-pass-diarization-speaker-identity.md` | Two-pass speaker diarization: Pass 1 (ECAPA-TDNN + online k-NN, live, low-latency); Pass 2 (AHC + VB-HMM/VBx, post-meeting, source-of-truth); rolling centroid profile optimization; reference papers |
| **0016** | `0016-auth-identity-bridge.md` | Microsoft Entra ID / MSAL OAuth2 authentication; objectId→voice-centroid DB bridge; biometric consent / privacy / compliance section; KB permission gating |
| **0017** | `0017-knowledge-base-compiler.md` | Team knowledge base: Git source-of-truth (lane 2 curated), content-addressed lane 1 (raw captures), compiled disposable layers (Postgres / Redis / vector DB / S3), permission compilation invariants, agent-artifact inherited permissions, compile flow |
| **0018** | `0018-three-repo-topology.md` | Formal three-repo split: `yap-desktop` (thin client), `yap-server` (GB-class server + IaC), `yap-knowledge` (Git KB source-of-truth); optional future `yap-contracts`; mapping of current `cohere-transcribe-local` workspace |

---

## Existing ADRs to amend

| ADR | Amendment needed |
|-----|-----------------|
| **0001** dual STT backends | Add amendment note: team profile's live path is server-hosted streaming ASR; solo profile retains today's local Moonshine. The dual-model split principle is preserved. |
| **0002** CrispASR unified STT runtime | Add amendment note: in team profile, model residency and the moonshine-XOR-cohere rule move to the server-side workload router; CrispASR on the client is demoted to offline/degraded-mode fallback; on-prem server is "our hardware," not cloud. |
| **0003** long-term voice architecture | Add amendment note: "local-first" layers (L1–L7) are reframed as the **solo profile**; team profile relocates STT, LLM pool, and L3 workers server-side; framing note that on-prem GPU ≠ cloud. |
| **0004** background diarization / OKF agents | Add amendment note: diarization design partially superseded by ADR 0015 two-pass approach; WeSpeaker → ECAPA-TDNN; online k-NN replaces vault-first pass; AHC+VBx replaces spectral clustering; worker subprocess moves to server in team profile. |
| **0005** llama-server agents | Add amendment note: in team profile, llama-server moves to the LLM pool on the GB-class server node; solo profile keeps bundled llama-server unchanged; CPU-first rule remains for solo. |
| **0006** Silero VAD / state machine | Add amendment note: in team profile, model-residency state machine (moonshine XOR cohere) moves to the server-side workload router; client orchestrator becomes the server-connector state machine; Silero VAD stays client-side for local chunk endpointing. |
| **0009** knowledge worker IPC protocol | Add amendment note: in team profile, the knowledge worker subprocess is replaced by the server-side KB compiler service; IPC protocol is superseded by the `yap-server` REST/gRPC APIs defined in ADR 0017; solo profile keeps the TCP JSON-lines protocol. |
| **0010** OKF conversation schema | Add amendment note: in team profile, conversations enter Lane 1 (append store) not written directly to OKF; the KB compiler normalises to OKF markdown and commits to `yap-knowledge` for curated content; file schema (frontmatter fields) is unchanged. |
| **0011** vector RAG retrieval | Add amendment note: in team profile, SQLite + `sqlite-vec` is replaced by a server-side vector DB (Milvus/pgvector-class) as a compiled disposable layer; schema, chunking strategy, and confidence gate are preserved; solo profile keeps the local SQLite approach. |
| **0012** MCP server surface | Add amendment note: in team profile, the MCP server runs as a sidecar of `yap-server` (not the desktop), exposing the compiled KB view filtered by permission; solo profile keeps the local stdio MCP server. |

---

## Phase roadmap additions

Add the following phases to the master roadmap in `VOICE-OS-ARCHITECTURE.md`. Existing phase IDs (0–7e, 7+, A–D) are **unchanged**; the server-tier track uses phase numbers 8–12.

| Phase | Track | Deliverable | ADR |
|-------|-------|-------------|-----|
| **8** | server | Server tier stand-up: GB-class server node, workload router, streaming ASR pool (Moonshine GPU), Cohere batch pool (concurrent workers); thin-client audio streaming connector | ADR 0014, 0018 |
| **9** | server | Auth: Entra ID / MSAL sign-in, objectId→voice-centroid DB bridge, KB permission gating | ADR 0016 |
| **10** | server | Two-pass diarization: ECAPA-TDNN live pass (server-side), AHC+VBx post-meeting pass; rolling centroid optimization | ADR 0015 |
| **11** | server | KB compiler service: Lane 1 raw-capture append store, Lane 2 Git curated path, Postgres ledger, Redis permission cache, vector index, permission-filtered OKF view | ADR 0017 |
| **12** | repo | Three-repo migration: `yap-desktop` / `yap-server` / `yap-knowledge` formal split; `yap-contracts` deferred | ADR 0018 |

---

## Top-level doc updates

| File | Change |
|------|--------|
| `docs/VOICE-OS-ARCHITECTURE.md` | (1) Add "Two deployment profiles" section. (2) Update pipeline charts to show thin-client shell + server tier. (3) Update master roadmap table with phases 8–12. (4) Update layer model to note server vs client boundary. (5) Update "Why it's a good idea" table for on-prem GPU rationale. (6) Update document map table. |
| `docs/adr/README.md` | Add ADRs 0014–0018 to the index table. |
| `docs/README.md` | No structural change required; it links to VOICE-OS-ARCHITECTURE.md which will be updated. |

---

## Open questions / decisions deferred

1. **Server-node network topology** — Is the server on a flat LAN, VPN-only, or mTLS-gated? Matters for the audio-streaming protocol choice (WSS/QUIC) and offline-fallback trigger. Deferred to ADR 0014 implementation phase.
2. **Audio-on-wire privacy** — Does the org need E2E encryption between client and DGX for HIPAA/GDPR compliance, or is TLS-in-transit sufficient? Deferred; ADR 0014 flags this as an open question.
3. **Biometric data jurisdiction** — Which data-protection regime applies (GDPR, CCPA, HIPAA)? Affects retention period and deletion SLA. Deferred to ADR 0016; org legal must confirm.
4. **Lane 1 migration threshold** — When does raw-capture volume trigger the move from a Git-based Lane 1 to content-addressed versioning? ADR 0017 specifies the design but leaves the threshold as a monitoring-driven decision.
5. **`yap-contracts` extraction timing** — Start shared API types inside `yap-server`; extract to a separate repo only when `yap-desktop` and `yap-server` have divergent release cadences. No specific trigger defined.
6. **SpeechBrain LID in team profile** — With server-side Moonshine streaming ASR and GPU resources, does SpeechBrain LID (ADR 0008) need to run client-side or server-side? ADR 0008 is unchanged for the solo profile; team profile deferred.
7. **Multilingual live streaming** — Server GPU removes the latency excuse against multilingual live; ADR for per-language streaming backends still deferred per ADR 0003 note.
8. **Voice biometric enrollment UX** — How does the first-enroll flow work? (Passive accumulation vs explicit "enroll voice" step?) ADR 0016 specifies consent but defers the UX flow.

---

## Checklist

- [x] Create `docs/adr/0014-server-tier-compute-topology.md`
- [x] Create `docs/adr/0015-two-pass-diarization-speaker-identity.md`
- [x] Create `docs/adr/0016-auth-identity-bridge.md`
- [x] Create `docs/adr/0017-knowledge-base-compiler.md`
- [x] Create `docs/adr/0018-three-repo-topology.md`
- [x] Amend `docs/adr/0001-dual-stt-backends.md`
- [x] Amend `docs/adr/0002-crispasr-unified-stt-runtime.md`
- [x] Amend `docs/adr/0003-long-term-voice-architecture.md`
- [x] Amend `docs/adr/0004-background-diarization-okf-agents.md`
- [x] Amend `docs/adr/0005-llama-server-agents.md`
- [x] Amend `docs/adr/0006-silero-agents-state-machine.md`
- [x] Amend `docs/adr/0009-knowledge-worker-protocol.md`
- [x] Amend `docs/adr/0010-okf-conversation-schema.md`
- [x] Amend `docs/adr/0011-vector-rag-retrieval.md`
- [x] Amend `docs/adr/0012-mcp-server-surface.md`
- [x] Update `docs/VOICE-OS-ARCHITECTURE.md`
- [x] Update `docs/adr/README.md` index
