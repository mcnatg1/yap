# Spec: Server Tier MVP

**Status:** Canonical Phase 3 boundary and isolated Phase 4 Cohere reference pool implemented; connected upload/WSS, queue drain, auth, and persistent service deployment remain deferred
**Scope:** Stand up the first server path while keeping this repo as the MVP monorepo.

The server tier introduces `yap-server` on an org-owned GB-class server node.
The desktop remains the product surface. Phase 3 implements the wire contract
and capability-health boundary. Phase 4 implements one transient, server-internal
Cohere batch path through a bounded router/pool and isolated GPU worker. It does
not yet connect the desktop contract to that worker.

## Repo Layout

```text
cohere-transcribe-local/
  desktop/               installed client and local Nemotron fallback
  server/                service/API staging area
  infra/yap-server-node/ GB-class host setup and firewall/runbook scripts
  docs/                  ADRs, specs, runbooks
```

This stays a monorepo through canonical Phase 9. Split into `yap-desktop`, `yap-server`, and `yap-knowledge` in canonical Phase 10 after the server is deployable and access boundaries are real.

`server/` starts small and tracks the future service shape without adding a framework:

```text
server/
  openapi/              HTTP contract + live WSS event notes
  src/yap_server/
    api/                health and future HTTP/WSS entrypoints
    workload_router/    bounded live/batch queues, fairness, and pool dispatch
    pools/              isolated Cohere batch reference pool; streaming later
    schemas/            request/event/job shapes, no model weights
    config/             environment/config parsing
  tests/                contract, API, workload-router tests
```

Avoid top-level `models/` and `workers/` during MVP. Model files live on the
node, outside Git. The Phase 4 worker process is container-isolated behind the
pool interface; persistent deployment topology remains deferred.

## First Build Slice

| Slice | Minimum outcome |
|-------|-----------------|
| API contract | Health, live WSS shape, batch upload/job shape documented in `server/` |
| Router | Health contract and live/batch route selection protected by tests |
| Desktop connector | Settings URL + reachability check + fallback state transitions |
| Host setup | `infra/yap-server-node/setup-server.sh` can prepare the node without opening app ports by default |

## Implemented Phase 3 Boundary

- `server/openapi/openapi.json` and `server/openapi/live-events.schema.json` define machine-readable health, future job/upload, and future live-event contracts.
- `python -m yap_server` implements bounded loopback `GET /v1/health`; advertised batch/live/job-status capabilities remain `false`.
- The Rust desktop connector validates configured origins, performs bounded health checks, fails closed on incompatible responses, rejects stale generations, and cancels retries when settings change.
- The desktop SQLite ledger owns imported-job IDs, status, attempts, source provenance, cancellation intent, restart recovery, and idempotent legacy migration.
- React renders Rust snapshots/events. It does not own queue execution through localStorage.

This boundary intentionally does not implement job/chunk handlers, automatic
queue drain, WSS transport, authentication/token validation, or server
processing. The separate Phase 4 reference pool does not change the Phase 3
health capabilities or make those contract-only routes executable.

## Implemented Phase 4 Reference Slice

- A bounded in-memory workload router enforces total and per-owner admission,
  bounded live priority without batch starvation, round-robin owner fairness,
  and explicit target availability under ADR 0023.
- A bounded thread-backed batch pool dispatches one reference job to a
  non-root, networkless, read-only container with dropped capabilities,
  `no-new-privileges`, resource and output ceilings, read-only inputs, an
  explicitly non-executable general `/tmp`, and a private executable Triton
  cache. Unique naming plus unconditional force-remove cleanup prevents a
  killed or timed-out Docker client from leaving the worker container behind.
- `server/model-pools.lock.json` pins the canonical Cohere model, its public
  byte-distribution revision, all model hashes, the licensed WER fixture, the
  exact NGC ARM64 base digest, Python 3.12, Torch/CUDA identities, and every
  overlay wheel version.
- The transient GB10 gate verifies artifacts, image architecture/revision,
  execution by the inspected raw image ID, router-to-pool dispatch, input/result
  audio identity, exact GB10 compute capability and BF16 execution, runtime
  identity, and maximum WER. It publishes final evidence only after observed
  listener/firewall/service state is unchanged and no Phase 4 container or
  worker remains.

This is a private reference vertical slice, not a production multi-user
router. Safe multi-worker capacity, a durable server queue, upload/drain, live
streaming, auth-derived owner identity, service supervision, observability,
and the application network edge remain later gates.

## Non-goals

- No separate `yap-contracts` repo.
- No Nx/Turborepo.
- No broad local batch fallback.
- No public internet exposure.
- No direct model/database ports exposed outside the server node.
- No claim that one short-file reference inference establishes 45-minute
  throughput or safe concurrent-worker capacity.

## Acceptance

- `README.md` explains the MVP monorepo rule.
- `server/README.md` defines where server-tier code lands.
- `python -m unittest discover -s server/tests -p "test_*.py"` passes with `PYTHONPATH=server/src`.
- `infra/yap-server-node/setup-server.sh` stays idempotent and syntax-valid.
- ADR 0018 still records the canonical Phase 10 three-repo split.
- Connector integration covers healthy Python health plus refused, timeout, malformed, auth-required, disabled, stale-configuration, and retry-cancellation behavior.
- Native restart restores the same Rust-owned queued job without WebView localStorage authority.
- The Phase 4 lock/runtime contract tests pass under Python 3.12.
- A clean checked-head GB10 gate produces immutable result/evidence JSON for
  the licensed fixture with WER at or below `0.12` and leaves no persistent
  process, listener, or firewall change.
