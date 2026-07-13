# yap-server staging

This directory is the MVP staging area for the future `yap-server` repo.

It remains part of the MVP monorepo. Do not split the server or contracts into
another repository before the canonical Phase 10 boundary.

Keep it boring until there is a real service:

- API contracts live here first, likely `openapi/`.
- Router/service code lives here only when it has tests.
- Model worker code lives here only after the router contract exists.
- Host setup stays in `../infra/yap-server-node/`.
- Shared desktop/server contracts stay here until type drift proves a separate `yap-contracts` repo is worth it.

The server tier grows inside this shape:

```text
server/
  README.md
  openapi/
    README.md
    openapi.json            # Phase 3 health + later HTTP contracts
    live-events.schema.json # contract only until live transport ships
  src/
    yap_server/
      api/
      workload_router/
      pools/
      schemas/
      config/
  tests/
    README.md
    contract/
    api/
    workload_router/
```

Use `workload_router/` instead of vague `router/`. Use `schemas/` for API and message shapes. Do not add a repo `models/` directory; runtime model files belong on the server node, not in Git.

## Phase 3 contract boundary

`openapi/openapi.json` and `openapi/live-events.schema.json` are the normative
machine-readable wire contracts. Their presence does not mean every route is
implemented:

| Method | Path | Phase 3 behavior | Later owner |
|--------|------|------------------|-------------|
| `GET` | `/v1/health` | Implemented | Server process |
| `POST` | `/v1/jobs` | Contract only | Phase 5 upload intake |
| `GET` | `/v1/jobs/{jobId}` | Contract only | Phase 5 job status |
| `DELETE` | `/v1/jobs/{jobId}` | Contract only | Phase 5 cancellation |
| `PUT` | `/v1/jobs/{jobId}/chunks/{trackId}/{sequenceStart}-{sequenceEnd}` | Contract only | Phase 5 resumable upload |
| `POST` | `/v1/jobs/{jobId}/commit` | Contract only | Phase 5 upload commit |
| `GET` upgrade | `/v1/live` | Event schema only | Phase 5 WSS streaming |

Phase 3 health advertises `batchJobs`, `liveStreaming`, and `jobStatus` as
`false`. Upload handlers, job persistence, a WebSocket runtime, queue drain,
authentication, token validation, inference, and diarization are not present.

Contract JSON fields use camelCase. Immutable manifest and server enum values
use snake_case. The React `RecordingJobView` values are an explicit projection,
not alternate wire values.

Chunk uploads use `application/octet-stream` raw `pcm_s16le` bytes. The logical
idempotency key and the SHA-256 byte identity are separate: the same key and
hash is replay success, while the same key with a different hash is a 409
`CONTENT_IDENTITY_CONFLICT`. Job and chunk requests do not accept tenant or
owner-subject fields; those values become server-derived only after token
validation exists.

## Local checks

```powershell
$env:PYTHONPATH = "server/src"; python -m unittest discover -s server/tests -p "test_*.py"
```

Run only the wire-contract tests while editing the JSON documents:

```powershell
$env:PYTHONPATH = "server/src"; python -m unittest server.tests.contract.test_contract -v
```

## Run the Phase 3 health service

The service uses Python's bounded, single-request-at-a-time `HTTPServer` and has
no runtime dependencies. It binds to loopback by default:

```powershell
$env:PYTHONPATH = "server/src"
python -m yap_server
Invoke-RestMethod http://127.0.0.1:18765/v1/health
```

`YAP_SERVER_HOST` and `YAP_SERVER_PORT` override the address. A wildcard or
non-loopback host is rejected unless the process explicitly sets
`YAP_SERVER_ALLOW_PRIVATE_BIND=1`. Binding does not change firewall rules; the
server-node runbook keeps port 18765 tunnel-only by default.

Only `GET /v1/health` is implemented. Contract-only job, chunk, commit, and live
routes return a stable `501 NOT_IMPLEMENTED` JSON error. Request bodies are
capped at 1 MiB before any body read. Each accepted request has a two-second
wall-clock deadline, so slow-drip input cannot extend the single-request server
indefinitely. The service accepts HTTP/1.0 and HTTP/1.1 only.

Skipped for now: Nx/Turborepo, package workspace wiring, framework/server dependencies, checked-in model weights, and fake GB300 profiles.
