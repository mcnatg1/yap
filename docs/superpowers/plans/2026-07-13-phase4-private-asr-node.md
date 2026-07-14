# Phase 4 Private ASR Node Implementation Record

**Status:** Implementation candidate; final clean-head Phase 4 gate and reviewed
PR remain pending.

**Branch:** `feat/phase4-private-asr-node`

**Canonical owner:** Server (`server/` and `infra/yap-server-node/`)

**Merge/expiry target:** The focused Phase 4 private-ASR-node PR.

**Delete/archive condition:** Archive this working record after the Phase 4 PR
merges and its final checked-head gate evidence is preserved in the PR and ADR
implementation audit. Delete it only if a later durable implementation record
supersedes those links and evidence.

**Scope:** Prove one real private-server batch-ASR path on the DGX Spark GB10
without connecting or widening the Phase 3 client-facing service boundary.

## Architecture Authority And Phase Boundary

Applied decisions:

- [ADR 0014](../../adr/0014-server-tier-compute-topology.md) owns the private
  server topology and Phase 4 router/model-pool boundary.
- [ADR 0018](../../adr/0018-three-repo-topology.md) keeps the MVP in the staged
  monorepo; the repository split remains Phase 10.
- [ADR 0019](../../adr/0019-local-streaming-model-selection.md) keeps Nemotron
  as the desktop-local offline/degraded fallback and not imported-file ASR.
- [ADR 0021](../../adr/0021-http3-secure-edge-transport.md) keeps the future
  secure edge gated; Phase 4 opens no external listener or firewall rule.
- [ADR 0023](../../adr/0023-bounded-live-priority.md) amends ADR 0014's
  absolute priority wording with bounded live preference.

Superseded or historical details intentionally ignored:

- Local-heavy runtime/model details in ADRs 0002, 0005, and 0009 do not select
  the team server runtime.
- Historical phase numbering does not move upload/drain, live transport,
  authentication, or repository splitting into this phase.
- ADR 0014's original absolute live-priority sentence is preserved as history
  and amended by ADR 0023 rather than silently rewritten.

The exact acceptance path is the focused Python 3.12 model/runtime/router test
set, compilation and diff checks, review/security closure, followed once by the
clean checked-head local/native/server/GB10 matrix and
`infra/yap-server-node/phase4-asr-gate.sh`. Phase 4 ends at one transient,
isolated batch worker plus an unconnected reference router. The Phase 5 and
enterprise decisions listed at the end of this record remain deferred.

## Selected Runtime And Model

| Concern | Locked choice |
|---|---|
| Server batch model | `CohereLabs/cohere-transcribe-03-2026` |
| Canonical revision | `b1eacc2686a3d08ceaae5f24a88b1d519620bc09` |
| Public byte distribution | `ZoOtMcNoOt/yap-cohere-transcribe-03-2026` |
| Distribution revision | `7fd11d290a33580014fc2d347ea81aa2670c2ea9` |
| Runtime base | `nvcr.io/nvidia/pytorch:26.06-py3` |
| ARM64 base digest | `sha256:dcae8df08ef61b019b8eb109113428cba4ef0e37484c6e722406150dd5ada759` |
| Python | 3.12 |
| NVIDIA Torch build | `2.13.0a0+8145d630e8.nv26.06` |
| CUDA toolkit / Torch CUDA | 13.3.0 / 13.3 |

The canonical model identity remains Cohere. The public distribution is only a
credential-free byte-delivery path and is not a renamed model. Transformers may
show `transformers/models/parakeet/modeling_parakeet.py` in an internal stack
trace because Cohere's implementation reuses that encoder module. Yap does not
select, download, advertise, or route to a Parakeet model.

## Executable Slice

```text
licensed PCM16 fixture
  -> bounded WorkloadRouter
  -> bounded BatchAsrPool
  -> ContainerBatchAsrWorker
  -> isolated Cohere CUDA/BF16 inference
  -> validated and atomically published result/evidence JSON
```

The public modules and their responsibilities are:

- `server/model-pools.lock.json`: one authority for runtime, canonical and
  distribution model identities, languages, artifact hashes, limitations, and
  licensed fixture.
- `server/src/yap_server/workload_router/router.py`: bounded total/per-owner
  admission, bounded live priority, owner round-robin, duplicate rejection,
  and available-target dispatch. When both targets stay ready, the reference
  router forces one batch dispatch after at most eight consecutive live jobs.
- `server/src/yap_server/pools/batch_asr.py`: bounded pool, explicit job
  language/punctuation, container command, output validation, and atomic host
  publication.
- `server/src/yap_server/pools/batch_asr_worker.py`: bounded mono PCM16/16 kHz
  input, local-only model loading, Cohere generation/decoding, and runtime/model
  attestation.
- `server/src/yap_server/pools/phase4_gate.py`: immutable artifact checks,
  ARM64/revision image inspection, router-to-pool execution, CUDA assertion,
  WER threshold, and atomic evidence.
- `infra/yap-server-node/phase4-asr-gate.sh`: clean-head build/run entrypoint
  with no daemon, published port, or firewall mutation.

## Process-Safety Boundary

Each inference job runs with:

- the invoking host's explicit non-root UID and GID;
- no container network;
- a read-only root filesystem;
- all Linux capabilities dropped and `no-new-privileges` enabled;
- bounded PID, shared-memory, general temporary-storage, and Triton-cache
  allocations;
- a 96 GiB memory ceiling, no swap beyond that ceiling, and a 16-CPU ceiling;
- read-only model and audio mounts;
- offline Hugging Face/Transformers settings;
- an explicitly non-executable general `/tmp`;
- a separate mode-0700 executable `/triton-cache` tmpfs owned by the worker
  identity, because the pinned NVIDIA Torch build JIT-compiles and maps a small
  Triton helper;
- a one-MiB ceiling on each captured worker output stream;
- a unique per-job container name and unconditional force-remove read-back, so
  killing a timed-out Docker client cannot leave its container running;
- result publication only after the worker's audio SHA-256, duration, sample
  rate, model, runtime, language, and punctuation attestations match the job.

The dedicated Triton cache is required executable behavior, not broad writable
or executable container storage. The focused failure with a wholly no-exec
temporary filesystem ended at `failed to map segment from shared object`; the
dedicated cache fixed that while preserving the rest of the containment.

## Dependency And License Closure

The NGC base supplies Torch/CUDA and most scientific dependencies. The Cohere
worker adds exactly the 14-package resolver delta in
`server/runtime/asr/requirements.lock`; every wheel is version- and SHA-256
locked. `librosa` and `soundfile` are required by the actual Cohere feature
extractor even when Yap passes a NumPy waveform. The runtime notice records all
overlay licenses plus the LGPL notices for bundled `libsndfile` and libsoxr;
the reference image also carries the complete Apache-2.0 text governing the
Cohere model.

The Docker build fails on:

- an incorrect wheel hash or unavailable ARM64 wheel;
- a new `pip check` failure beyond the one exact known NGC base diagnostic;
- inability to import the Cohere/audio stack;
- Python, Torch, CUDA, or any overlay-version drift.

The build context excludes bytecode, tests, local environments, logs,
databases, keys, model weights, and private runtime state. Model acquisition
accepts only the locked byte count and SHA-256, caps the stream before writing
past that count, and follows redirects only to approved Hugging Face HTTPS
hosts. The gate inspects the candidate tag once and executes the returned raw
image ID, not the mutable tag.

## Focused Pre-Gate Evidence

This evidence proves the implementation seam while the one-time clean-head
phase gate remains pending:

- Focused local checks: 40 model-pool tests and 17 runtime/router tests passed;
  compilation and `git diff --check` passed.
- An earlier pre-hardening focused subset passed on the GB10 under Python 3.12;
  the final exact-head matrix and transient inference gate remain pending and
  will supersede that development evidence.
- Derived ARM64 image:
  `sha256:c513d6c39cb8ad1ce5e16ee650b46e3001318fef017af2ca17d7bec1f8399446`.
- Public worker path ran the licensed 7.435-second fixture on `NVIDIA GB10`,
  compute capability 12.1, in BF16/CUDA.
- Model load: 27,038 ms; inference: 1,915 ms.
- Transcript WER: `0.0` against the locked golden transcript.
- Returned runtime identity: Python 3.12.3, NVIDIA Torch
  `2.13.0a0+8145d630e8.nv26.06`, Torch CUDA 13.3, and all 14 locked overlay
  versions.

This is not final checked-head evidence and is not a long-file or concurrency
benchmark.

## Final Phase 4 Gate

Before the PR is eligible to merge:

1. Finish code, notices, runbook, ADR, architecture, and status reconciliation.
2. Review the complete diff and resolve correctness/security findings.
3. Commit a clean implementation candidate.
4. Run `infra/yap-server-node/phase4-asr-gate.sh` exactly once on that exact
   clean SHA in the disposable GB10 candidate checkout.
5. Record the immutable result/evidence digest only after before/after
   listener, firewall-policy observation, and Yap service-unit snapshots match
   and no Phase 4 container or worker process remains. The firewall observation
   uses effective UFW status when narrowly authorized and otherwise compares
   persistent UFW configuration metadata plus unit state. Raw host snapshots
   stay in the gate's temporary directory; final evidence contains only hashes
   and observed pass/fail facts. The checked-head evidence directory must not
   exist before the run; publication reserves it once and refuses every
   overwrite or silent reuse.
6. Add only evidence/status reconciliation after the gate; do not change the
   gated executable tree.
7. Open a focused PR, use hosted CI when available, and merge only after the
   checked PR head SHA is green.

## Explicit Phase 5 And Enterprise Handoffs

Phase 4 does not implement or claim:

- desktop upload/drain or automatic queue execution;
- job/chunk/commit HTTP handlers or advertised batch capability;
- WSS live streaming or a streaming ASR pool;
- authenticated owner derivation, Entra/MSAL, or token validation;
- durable server queues, cancellation/recovery, warm workers, safe concurrency,
  service supervision, monitoring, or long-recording capacity;
- an external app listener, TLS certificate, DNS, corporate firewall policy,
  ZPA app segment, or enterprise deployment.

Those network and enterprise controls require the documented IT handoff; a
developer-owned substitute is not acceptable.
