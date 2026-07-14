# yap-server staging and private reference runtime

This directory is the MVP staging area for the future `yap-server` repo.

It remains part of the MVP monorepo. Do not split the server or contracts into
another repository before the canonical Phase 10 boundary.

Keep the interfaces narrow while the private server path becomes real:

- API contracts live here first, likely `openapi/`.
- Router/service code lives here only when it has tests.
- Model worker code is isolated from the health process and must remain pinned,
  bounded, and independently gated.
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
      pools/                 # bounded Phase 4 reference worker and pool
      schemas/
      config/
  runtime/asr/               # pinned ARM64 image recipe and notices
  model-pools.lock.json      # exact runtime/model/fixture authority
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
`false`. Upload handlers, server-side job persistence, a WebSocket runtime,
queue drain, authentication, token validation, and diarization are not present.
The Phase 4 reference worker below is deliberately not exposed through those
contract-only routes, so health reachability cannot imply connected ASR.

Contract JSON fields use camelCase. Immutable manifest and server enum values
use snake_case. The React `RecordingJobView` values are an explicit projection,
not alternate wire values.

Chunk uploads use `application/octet-stream` raw `pcm_s16le` bytes. The logical
idempotency key and the SHA-256 byte identity are separate: the same key and
hash is replay success, while the same key with a different hash is a 409
`CONTENT_IDENTITY_CONFLICT`. Job and chunk requests do not accept tenant or
owner-subject fields; those values become server-derived only after token
validation exists.

## Phase 4 private batch-ASR reference slice

Phase 4 adds one executable server-internal vertical slice without turning the
Phase 3 health process into a production service:

- `model-pools.lock.json` pins the canonical Cohere model and revision, the
  public byte-distribution revision, every deployed artifact hash, the licensed
  speech fixture, the complete model license text, and the exact ARM64 runtime
  identity.
- `runtime/asr/Dockerfile` uses
  `nvcr.io/nvidia/pytorch:26.06-py3` by immutable digest, Python 3.12, the
  NVIDIA Torch/CUDA build from that image, and a hash-locked resolver-minimal
  Python overlay.
- `WorkloadRouter` provides bounded total/per-owner admission, bounded live
  priority without batch starvation, round-robin owner fairness, and explicit
  pool dispatch in memory.
- `BatchAsrPool` provides a bounded thread-backed queue. Its container adapter
  runs each job non-root with no network, a read-only root filesystem, dropped
  capabilities, `no-new-privileges`, memory/CPU/PID/output ceilings, read-only
  model/audio mounts, an explicitly non-executable `/tmp`, and only a private
  executable tmpfs for Triton JIT output. Every run has a unique container name
  and an unconditional force-remove cleanup check.
- `phase4_gate.py` connects router -> pool -> isolated worker, verifies the
  immutable model and licensed fixture, executes the inspected raw image ID,
  requires input/result audio identity plus exact GB10/compute-capability/BF16
  runtime attestation, and enforces the fixture WER threshold. The wrapper
  publishes results atomically only after listener, firewall, Yap service-unit,
  container, and worker-process read-back passes.

The reference slice is not an upload endpoint, automatic desktop queue drain,
authenticated session, persistent server process, external listener, or
multi-worker capacity claim. Those remain separate Phase 5 and production
deployment gates.

## Local checks

```powershell
$env:PYTHONPATH = "server/src"; python -m unittest discover -s server/tests -p "test_*.py"
```

Run only the wire-contract tests while editing the JSON documents:

```powershell
$env:PYTHONPATH = "server/src"; python -m unittest server.tests.contract.test_contract -v
```

The clean-head GB10 gate is run from the private node, not from normal local or
hosted CI:

```bash
YAP_CHECKED_HEAD=<full-git-sha> \
YAP_PHASE4_MODEL_DIR=<private-model-directory> \
YAP_PHASE4_EVIDENCE_DIR=<private-evidence-directory> \
bash infra/yap-server-node/phase4-asr-gate.sh
```

The gate builds and runs a transient container only. It does not install a
service, publish a port, or change the host firewall. Raw host snapshots exist
only in its temporary directory; final evidence stores hashes and observed
facts, not listener or firewall details.

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

Skipped for now: Nx/Turborepo, package workspace wiring, framework/server
dependencies, checked-in model weights, persistent worker deployment, and fake
GB300 profiles.
