# Spec: Phase 8 yap-server MVP

**Status:** Draft
**Scope:** Stand up the first server path while keeping this repo as the MVP monorepo.

Phase 8 introduces `yap-server` as a private service on an org-owned GB-class server node. The desktop remains the product surface; the server owns heavy inference, queues, and the future API contract.

## Repo Layout

```text
cohere-transcribe-local/
  desktop/               installed client and local Moonshine fallback
  server/                Phase 8 service/API staging area
  infra/yap-server-node/ GB-class host setup and firewall/runbook scripts
  docs/                  ADRs, specs, runbooks
```

This stays a monorepo through MVP. Split into `yap-desktop`, `yap-server`, and `yap-knowledge` in Phase 12 after the server is deployable and access boundaries are real.

`server/` starts small and tracks the future service shape without adding a framework:

```text
server/
  openapi/              HTTP contract + live WSS event notes
  src/yap_server/       service code when Phase 8 begins
    api/                HTTP/WSS entrypoints
    workload_router/    queues, fairness, backpressure, pool dispatch
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
| Router | One service entrypoint that can accept authenticated health and stub job requests |
| Desktop connector | Settings URL + reachability check + fallback state transitions |
| Host setup | `infra/yap-server-node/setup-server.sh` can prepare the node without opening app ports by default |

## Non-goals

- No separate `yap-contracts` repo.
- No Nx/Turborepo.
- No broad local batch fallback.
- No public internet exposure.
- No direct model/database ports exposed outside the server node.

## Acceptance

- `README.md` explains the MVP monorepo rule.
- `server/README.md` defines where Phase 8 code lands.
- `infra/yap-server-node/setup-server.sh` stays idempotent and syntax-valid.
- ADR 0018 still records the Phase 12 three-repo split.
