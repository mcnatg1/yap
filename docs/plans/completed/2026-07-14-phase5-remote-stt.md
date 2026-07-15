# Phase 5 Remote STT Implementation Record

**Status:** Completed, reviewed, gated, and merged. Exact PR head
`4771d9be60562fa009ccecbcd3c7111b699883a5` passed the one-time complete
Phase 5 matrix and hosted checks, then merged as
`b6677631b2cc8283f0f6466622f2dfa7cfdb38f6` on 2026-07-15. The body below is
the implementation/gate contract preserved as historical evidence, not active
work.

**Branch:** `feat/phase5-remote-stt`

**Canonical owners:** Desktop durable job path (`desktop/`) and private batch
service (`server/`).

**Scope:** Deliver one real imported-recording batch-ASR vertical slice through
Yap's documented create/upload/commit/status/result contract. Preserve local
live fallback. Keep server live streaming, authentication, enterprise edge,
diarization, and multi-user production service work in their accepted later
phases.

## Architecture Authority

This phase applies the accepted ADRs without pulling later concerns forward:

- [ADR 0014](../../adr/0014-server-tier-compute-topology.md) owns the thin
  desktop, private GB-class server, workload-router, and model-pool topology.
- [ADR 0016](../../adr/0016-auth-identity-bridge.md) still gates authenticated
  tenant/owner derivation in Phase 7. Until then, Phase 5 batch audio is allowed
  only over a manually established loopback SSH tunnel in the development
  profile; no client-supplied owner becomes authoritative.
- [ADR 0020](../../adr/0020-meeting-capture-diarization-authority.md) owns
  server-authoritative result revisions. This slice publishes transcript text
  with no speaker result and no invented word alignment.
- [ADR 0021](../../adr/0021-http3-secure-edge-transport.md) keeps TLS/QUIC,
  HTTP/3, UDP exposure, and production TCP fallback at the later secure-edge
  gate. Phase 5 opens no external application listener or firewall rule.
- [ADR 0023](../../adr/0023-bounded-live-priority.md) remains the router
  scheduling authority. Reusing its bounded router with only the batch target
  ready does not claim that live server ASR exists.
- [ADR 0018](../../adr/0018-three-repo-topology.md) keeps the MVP in this
  monorepo until the later repository-split gate.

Historical local-heavy model/runtime ADRs do not select the team-server ASR
runtime. The [Phase 5 runtime evaluation](../../research/2026-07-14-phase5-asr-runtime-evaluation.md)
keeps the already verified NVIDIA PyTorch/Transformers/BF16 path as the
executable baseline and makes vLLM a measured challenger, not an unverified
replacement.

## Executable Vertical Slice

```text
native canonical WAV source
  -> Rust-owned PCM16/16 kHz validation, extraction, and immutable preparation
  -> immutable Yap spool + durable SQLite ledger
  -> create + idempotent bounded chunk upload + commit
  -> private loopback yap-server
  -> bounded router + isolated Cohere batch worker
  -> status + immutable server-authoritative result
  -> verified local result revision + desktop History projection
```

### Desktop ownership

The native desktop, not the WebView, owns the lifecycle:

- an admitted external source is read without following reparse points and is
  never deleted by cancellation, retry, or retention;
- preparation admits only an already-canonical mono PCM16/16 kHz RIFF/WAVE,
  extracts its PCM under exact physical/container bounds, records timing and
  hashes, emits one-MiB-or-smaller chunks, and atomically publishes the owned
  spool/manifest. MP3, M4A, video, compressed audio, resampling, and channel
  conversion remain outside this vertical slice and are not advertised by the
  native picker;
- the SQLite ledger persists preparation, chunk receipts, server origin,
  server job identity, retry state, cancellation intent, and final result path;
- create uses the SHA-256 of the exact canonical create request as its
  idempotency key; upload accepts only exact receipt identity; reconnect checks
  status before replaying commit;
- automatic retries are typed and bounded to six attempts. Invalid responses
  and exhausted retries become explicit user-visible job failures rather than
  infinite polling;
- cancellation is a durable detached outbox action and is drained before new
  upload work for the active origin. Retrying a failed remote job clears its
  former server binding, prepared chunks, and Yap-owned spool while preserving
  the external source;
- startup and lifecycle cleanup remove only exact Yap-owned preparation and
  quarantine directory shapes, including crash remnants, without following
  reparse points or touching similarly prefixed external paths;
- downloaded results are atomically published under a revisioned native
  directory and re-opened only after exact schema, authority, identity, hash,
  path, file-set, size, and transcript-byte validation; and
- the React queue and History are projections of native snapshots/events.
  Completed server jobs appear through the verified native result catalog, not
  renderer-owned state.

### Server ownership

The private service implements:

- `POST /v1/jobs` with exactly one create idempotency key;
- bounded, identity-checked PCM chunk upload;
- manifest-bound commit and one bounded router/pool dispatch;
- status, immutable result, and idempotent cancellation;
- restart-safe job state, create-key replay, chunk receipts, result recovery,
  and safe conversion of interrupted processing to a retryable terminal state;
- atomic result publication, including recovery when a result reached disk
  before its final state write and removal when cancellation won the race;
- finite meeting retention, startup and periodic expiry maintenance, immediate
  purge of cancelled/failed private audio at the safe lifecycle boundary,
  30-day completed-result retention, a 512-job retained-record cap, a
  four-hour PCM admission cap, and eight HTTP request workers; and
- dynamic health capabilities: batch/status become true only when the Linux
  Phase 5 runtime is actually enabled; live streaming remains false and
  `/v1/live` remains unimplemented.

Here, durable/restart-safe means process-crash recovery on the private node's
normally operating local filesystem. Phase 5 does not claim a power-loss
durability SLA, backup/restore, replicated storage, or disaster recovery; those
remain part of the later persistent multi-user service handoff.

The GPU runtime requires Linux, a non-root server identity, private mode-0700
storage, the immutable runtime/model lock, verified model artifacts, and a
custom Yap worker image whose revision label matches the exact checked head.
The NVIDIA PyTorch image is the pinned build base, not a directly runnable Yap
worker. The inspected immutable custom-image ID is transient, networkless,
read-only, capability-dropped, PID/CPU/memory bounded, and force-cleaned under
the existing Phase 4 containment boundary.

## Development Transport Boundary

The executable development path is intentionally narrower than the future team
deployment:

- server bind: `127.0.0.1:18765` only;
- Windows connector: `http://127.0.0.1:18765` only;
- transport: explicit `ssh -L` over either the direct private-Ethernet alias or
  one manually selected Wi-Fi SSH alias;
- no automatic network or alias failover; and
- SSH is the temporary access boundary. There is no Entra-derived Yap owner,
  TLS application endpoint, DNS identity, ZPA publication, or app firewall
  opening in this phase.

The exact commands and lifecycle rehearsal are in the
[server-node runbook](../../runbooks/yap-server-node-setup.md#phase-5-loopback-batch-development).
Private audio, transcripts, job stores, scan output, and host evidence stay out
of Git, CI artifacts, and PR attachments.

## Focused Development Verification

Focused checks are evidence for development decisions only. They do not replace
the final checked-head gate. The working set includes:

- frontend queue cancellation and native remote-history projection tests;
- Rust ledger restart/idempotency, typed retry, source/spool, cancellation,
  result verification, and catalog tests;
- Python create/upload/commit/status/result, bounded concurrency, cancellation
  race, restart recovery, retention, runtime, and API tests under Python 3.12;
- OpenAPI/schema contract checks; and
- compilation/diff checks appropriate to each focused change.

## One-Time Phase 5 Gate

Run the complete applicable local/native/server/GB10 matrix exactly once only
after the implementation, private security review, docs, and reviews are ready
on one checked candidate SHA. The gate must prove:

1. all tracked contract, frontend, Rust, native integration, server, runtime,
   infrastructure-policy, and provenance checks pass;
2. the desktop imports a licensed non-sensitive recording, prepares and uploads
   it, the actual GB10 worker returns the locked Cohere result, and History opens
   the verified server-authoritative revision;
3. the complete same-head Rust/Python matrix proves create and chunk replay
   survive client/server restart without duplicate jobs or bytes;
4. the native gate owns one explicit SSH alias and drops/restores its forward
   around the same immutable client job, observes `Retrying`, and resumes at the
   unchanged numeric-loopback origin;
5. the same-head lifecycle matrix proves cancellation wins every tested
   commit/result race and user retry creates a new server job while preserving
   the original source;
6. saturation, malformed data, timeout, worker failure, restart, storage limit,
   and retention paths fail closed without private-content logging;
7. Python 3.12, ARM64, GB10, image/model/runtime identities, WER, resource
   ceilings, and complete container/process/listener teardown are recorded; and
8. no external listener, application firewall rule, persistent service, model
   port, TLS/QUIC claim, or enterprise access claim appears.

The final candidate is immutable after the gate. Any code change creates a new
candidate and requires an explicit gate decision; do not silently inherit GB10
evidence from Phase 4 or an earlier Phase 5 SHA.

## Explicit Later Handoffs

Phase 5 does not invent enterprise infrastructure. The following remain gated:

| Owner/phase | Required handoff |
| --- | --- |
| Phase 7 product + identity/security | MSAL sign-in, Yap API audience/token validation, tenant-scoped `(tid, oid)` owner derivation, authorization, revocation, and audit behavior from ADR 0016 |
| IT/security/network | Synchronized server time, internal DNS, certificate source/trust, approved TLS termination, firewall source ranges, ZPA app segment/policy, App Connector placement/redundancy, and routing |
| Later server capacity | Persistent supervised service, durable multi-user queue/store, backup/deletion SLA, observability, measured long-recording and concurrency capacity, and failure recovery |
| Phase 10 secure edge/release | HTTP/3 versus TCP benchmark and promotion, production publication governance, enterprise deployment evidence, and eventual repository split |

Until those handoffs exist, the loopback SSH profile is a deliberate
development boundary and must not be re-described as production security.
