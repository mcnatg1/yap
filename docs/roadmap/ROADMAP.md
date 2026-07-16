# Yap Roadmap

The roadmap is ordered. Each phase uses a separate branch and focused reviewed
PR. Repository state, executable tests, and observed runtime behavior are the
completion authority.

The [Voice OS architecture](../VOICE-OS-ARCHITECTURE.md) is the long-term
full-system frame. This roadmap is the ordered delivery authority when that
frame contains alternate or historical sequencing.

## Delivered MVP foundation

| Phase | Delivered boundary |
| --- | --- |
| 0 | Architecture reset around thin desktop, private server, explicit local fallback, and queued/offline truth. |
| 1 | Desktop capture, durability, tray/island, playback/history, and imported-job foundations. |
| 2 | Explicit local Nemotron fallback model lifecycle and live transcription. |
| 3 | Server contracts, capability health, connector state/retry, durable desktop job ledger, canonical app-data/stock NSIS closure. |
| 4 | Bounded private router/pool and isolated Cohere GPU reference worker on the pinned Python 3.12/NVIDIA stack. |
| 5 | Real durable imported-recording batch-ASR slice through the desktop/server contract with verified native result publication. |

Evidence and limits are summarized in [current status](../CURRENT-STATUS.md).

## Current gate: Architecture Checkpoint A

Checkpoint A reviews the complete Phase 1–5 executable system before new
product scope:

- resolve correctness/security findings;
- establish one owner for lifecycle, persistence, window, result, retry, and
  cancellation state;
- remove dead/speculative machinery;
- decompose mixed responsibilities and justify cohesive size exceptions;
- make dependency direction explicit;
- reconcile provenance/licenses;
- measure before claiming efficiency gains; and
- organize current, normative, active, completed, historical, operational, and
  evidence documentation.

It adds no Phase 6 functionality. The active plan is
[Architecture Checkpoint A](../plans/active/2026-07-15-architecture-checkpoint-a.md).

## Accepted later phases

| Phase | Boundary | Exit direction |
| --- | --- | --- |
| 6 | Preprocessing | Audio normalization, VAD/chunk manifests, language identification, forced alignment, word timestamps, and durable retryable pipeline state. |
| 7 | Identity and access | Entra/MSAL client bridge, Yap API audience/token validation, tenant-scoped `(tid, oid)` ownership, purpose grants, authorization/revocation/audit behavior. |
| 8 | Meeting evidence | Anonymous speaker evidence, timestamped result revisions, benchmark gates, and purpose-authorized server reconciliation/naming. |
| 9 | Knowledge and agents | Pinned Google OKF profile, deterministic compiler, permission-safe relational/vector retrieval, governed agents/RAG/MCP. |
| 10 | Enterprise and release | IT-managed access/network hardening, secure-edge evaluation, production publication governance, audit/deploy evidence, and eventual repo split. |

Accepted ADRs remain requirements even when no premature implementation exists.
Do not treat an unchecked historical plan box as current backlog.

## Enterprise handoffs

The following are controlled by IT, security, networking, or enterprise
platform owners and cannot be invented by a developer branch:

- internal DNS and certificate issuance/trust;
- synchronized server time and approved host identity;
- enterprise firewall source ranges and policy;
- ZPA application segment, policy, App Connector placement, and redundancy;
- production identity registration, token audience, conditional-access and
  revocation behavior;
- persistent service supervision, backup/deletion SLA, monitoring, and capacity
  ownership; and
- enterprise deployment, publication, and audit approval.

Until those handoffs exist, the Phase 5 SSH-forward profile remains a narrow
development boundary, not production security.

## Phase working rules

1. Do not restart or duplicate merged work.
2. Keep phases independently reviewable and mergeable.
3. Use focused verification during development; run the complete applicable
   phase matrix once when the exact head is ready.
4. Resolve correctness/security findings before merge.
5. Preserve upstream provenance and verify licenses before reuse.
6. Update completion scores/status only after executable evidence exists.
7. Keep private scan material and sensitive runtime evidence out of Git, PRs,
   hosted logs, and public docs.
