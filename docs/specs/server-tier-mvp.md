# Spec: Server Tier MVP

**Status:** Canonical Phase 3 boundary implemented; upload/WSS, queue drain, auth, model pools, and server inference remain deferred
**Scope:** Stand up the first server path while keeping this repo as the MVP monorepo.

The server tier introduces `yap-server` as a private service on an org-owned GB-class server node. The desktop remains the product surface. Phase 3 implements the wire contract and capability-health boundary; the server's heavy inference and operational queues are future phases.

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
    workload_router/    live/batch route selection; later queues and pool dispatch
    pools/              streaming ASR and batch ASR pool adapters
    schemas/            request/event/job shapes, no model weights
    config/             environment/config parsing
  tests/                contract, API, workload-router tests
```

Avoid top-level `models/` and `workers/` during MVP. Model files live on the node; worker process topology can wait until the workload router contract is real.

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

This boundary intentionally does not implement job/chunk handlers, automatic queue drain, WSS transport, server ASR, authentication/token validation, model pools, or server processing.

## Non-goals

- No separate `yap-contracts` repo.
- No Nx/Turborepo.
- No broad local batch fallback.
- No public internet exposure.
- No direct model/database ports exposed outside the server node.

## Acceptance

- `README.md` explains the MVP monorepo rule.
- `server/README.md` defines where server-tier code lands.
- `python -m unittest discover -s server/tests -p "test_*.py"` passes with `PYTHONPATH=server/src`.
- `infra/yap-server-node/setup-server.sh` stays idempotent and syntax-valid.
- ADR 0018 still records the canonical Phase 10 three-repo split.
- Connector integration covers healthy Python health plus refused, timeout, malformed, auth-required, disabled, stale-configuration, and retry-cancellation behavior.
- Native restart restores the same Rust-owned queued job without WebView localStorage authority.
