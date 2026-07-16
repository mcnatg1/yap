# Architecture Checkpoint A File Inventory

**Implementation anchor:** `6e25cb7b076a768c73e2a37f6d73645d777af3b0`

**Inventory date:** 2026-07-15

**Method:** tracked files from `git ls-files`; physical line counts from the
checked-out files; module/import/symbol inspection for every listed file.

## Repository inventory

The anchor contains 829 tracked files. The exhaustive path partition is:

| Area | Tracked files | Notes |
| --- | ---: | --- |
| Documentation (`docs/`) | 75 | ADRs, plans, specs, research, runbooks, evidence, canonical status/architecture, and tracked historical records. |
| Desktop production (`desktop/src/` + `desktop/src-tauri/src/`) | 480 | 122 React/TypeScript files plus 358 Rust native files; Rust test submodules under `src` remain part of this path count. |
| Desktop dedicated tests (`desktop/tests/` + `desktop/src-tauri/tests/`) | 107 | 101 unit/Playwright/WDIO/release/fixture files plus 6 Rust integration files. |
| Desktop packaging/config/assets | 43 | Remaining tracked files under `desktop/`, including manifests, locks, icons, and packaging inputs. |
| Server production (`server/src/`) | 41 | Python API, bounded I/O, job, router, and pool/runtime modules. |
| Server tests | 52 | Portable contract/job/API/runtime/infra tests and licensed fixtures. |
| Server contract/runtime/config assets | 14 | Remaining tracked files under `server/`, including OpenAPI schemas, runtime locks/notices/licenses, and configuration. |
| Infrastructure | 4 | Server-node setup, environment example, and related policy material. |
| Hosted workflows | 3 | CI, disposable NSIS smoke, and staged draft release. |
| Other `.github` policy | 1 | Dependabot configuration. |
| Root product/repository files | 9 | README, product/design/changelog, provenance/notices, attributes/ignore, and Node version. |

Generated OpenAPI/schema files, lockfiles, icon binaries, and machine-produced
test output are excluded from mechanical size findings. Documentation is
classified separately from production/test size review. The generated files
present at the anchor are:

- `desktop/pnpm-lock.yaml`;
- `desktop/src-tauri/Cargo.lock`;
- `server/openapi/openapi.json`; and
- `server/openapi/live-events.schema.json`.

No vendored source, built `dist`, `target`, `node_modules`, private scan output,
model weight, or test-result directory is tracked. One reviewed ASR gate fixture,
`server/tests/fixtures/asr/2086-149220-0033.wav`, is intentionally tracked; its
public LibriSpeech source, CC BY 4.0 license, SHA-256, and golden transcript are
recorded in the fixture README and immutable model-pool lock. It is not private
user media.

## Retained files above 350 lines

Every hand-written production, test, script, or workflow file above 350 lines
was inspected. Production and inline-test line counts are separated where a
stable test-module boundary exists.

| Lines | File | Responsibility and owner | Decision |
| ---: | --- | --- | --- |
| 609 | `desktop/src-tauri/src/server_connector/state.rs` | Connector generation, in-flight request/retry state, snapshot transitions, and its transition tests. About 342 implementation + 267 inline-test lines. Depends on bounded connector results/config generations; emits connection snapshots. | Retain. One state-machine owner; extracting its tests is optional and would not reduce production coupling. |
| 537 | `desktop/src-tauri/src/audio/session.rs` | Validated session/track identity and metadata domain contract plus validation tests. About 394 implementation + 143 inline-test lines. | Retain. One foundational schema/validation reason to change; splitting individual value objects would scatter shared invariants. |
| 504 | `infra/yap-server-node/setup-server.sh` | Sourceable, fail-closed, idempotent server-node bootstrap: validate inputs, install baseline packages, create private directories/SSH policy, configure optional private Ethernet/firewall/log limits, suppress host noise, and report state. | Retain with strong justification. It is one copy-and-run operational transaction; splitting remote fragments would create version/copy skew. Its interface is covered by `server/tests/infra/test_server_node_setup.py`. |
| 495 | `server/src/yap_server/jobs/service.py` | Coordinates one server job transaction lifecycle across explicit store, upload, completion, runtime, artifact, and router/pool owners. | Retain. It is the domain service boundary; mechanisms are already below it and HTTP parsing is above it. Further splitting would create competing transaction coordinators. |
| 480 | `desktop/src-tauri/tests/audio_foundation.rs` | Dedicated cross-module audio durability/timeline integration scenarios with shared deterministic fixtures. | Retain. Test-only end-to-end contract matrix; production code is not mixed in. |
| 462 | `desktop/src-tauri/src/commands/history/catalog.rs` | Builds the native history projection and stable path identity; substantial test-only catalog fixtures/assertions follow the implementation. About 182 implementation + 280 test support/tests. | Retain. Production portion is below threshold and owns one projection boundary. |
| 460 | `desktop/src-tauri/src/server_connector/client.rs` | Bounded health HTTP adapter, response parsing/version/capability validation, offline projection, and adapter tests. About 171 implementation + 289 tests. | Retain. Production portion is small and cohesive; batch transport has already been separated. |
| 457 | `desktop/src-tauri/src/live/recordings/tests/transcripts.rs` | Transcript publication, revision, replacement-race, corruption, and catalog visibility scenarios. | Retain. Dedicated adversarial test matrix with no production responsibilities. |
| 432 | `desktop/src-tauri/src/live/recordings/tests/recovery.rs` | Recoverable capture repair/delete, identity replacement, catalog concurrency, and warning scenarios. | Retain. Dedicated recovery scenario matrix. |
| 425 | `desktop/src-tauri/src/live/devices.rs` | CPAL device enumeration/selection, permission classification, bounded preflight, and tests. About 237 implementation + 188 tests. | Retain. Production portion is below threshold and owns one OS device boundary. |
| 409 | `desktop/src-tauri/tests/model_download.rs` | Deterministic HTTP stall, cancellation, size/hash failure, cleanup, and atomic replacement integration tests. | Retain. One download lifecycle matrix and reusable local test servers. |
| 404 | `desktop/src-tauri/src/live/recordings/tests/catalog.rs` | Committed/legacy recording catalog admission, link/replacement rejection, path projection, and history visibility. | Retain. Dedicated catalog trust-boundary matrix. |
| 396 | `desktop/src-tauri/src/commands/live.rs` | Thin Tauri live/device/settings/overlay commands and conversion tests. About 334 implementation + 62 tests. | Retain. Just under the production decomposition threshold; commands delegate transitions and do not own runtime state. Reinspect if another command family is added. |
| 395 | `.github/workflows/release.yml` | Resolves an immutable default-branch commit, builds/seals/smokes one NSIS artifact, binds evidence, verifies the production environment, tags the same commit, and stages a draft release. | Retain with justification. The three jobs form one immutable publication transaction whose outputs cross GitHub trust boundaries. Policy/process/cache mechanics are tested in extracted release-contract modules; reusable workflow extraction would add another remote interface without reducing current ownership. |
| 391 | `desktop/src-tauri/src/live/recordings/tests/deletion_maintenance.rs` | Quarantine, intent reconciliation, bounded maintenance rotation, failure retry, and warning-priority scenarios. | Retain. Dedicated destructive-lifecycle safety matrix. |
| 390 | `desktop/src/components/ui/sidebar.tsx` | Compound accessible sidebar primitive: provider/context, trigger, inset, sections, menus, and variants. | Retain. One component-family API and styling/accessibility contract; product navigation behavior lives in `components/app/app-sidebar.tsx`. |
| 365 | `desktop/src-tauri/src/live/overlay_window.rs` | Native live-island identity, dimensions, monitor placement, visible region, system-window behavior, and geometry tests. About 307 implementation + 58 tests. | Retain. One native window authority; renderer presentation is separate. |
| 357 | `desktop/src-tauri/src/live/runtime/stream_session.rs` | Local ASR stream worker/finisher lifecycle, bounded drain, profile, and a small test-only adapter. About 342 production + 15 test-support lines. | Retain. One streaming-session lifecycle and below the 350-line production threshold; warmup, capture, ASR adaptation, state, and finalization are separate. |

There is no hand-written product module above 609 lines and no dedicated test
file above 480 lines. The only retained item above 500 that is not inline-test
heavy is the sourceable server bootstrap; its strong cohesion justification is
recorded above.

## Inspected files from 250 through 349 lines

The files below were inspected for responsibility, dependencies, owned state,
entry points, failure boundaries, test surface, and change coupling. They remain
below the mandatory 350-line decomposition threshold. The outcome column
records why no additional split is warranted now.

### Desktop native production

| Owner group | Files (lines) | Inspection outcome |
| --- | --- | --- |
| App/runtime composition | `app.rs` (329); `live/runtime/resources.rs` (341) | App composes Tauri lifecycle and resources; resources holds one live-runtime resource set. Feature behavior is delegated. |
| Connector configuration/state adapters | `server_connector/config/persistence.rs` (342); `server_connector/config.rs` (341); `server_connector/config/platform.rs` (306); `server_connector/core.rs` (291); `server_connector/desktop.rs` (263) | Atomic publication, validation/facade, platform no-follow I/O, stable policy/state, exclusive settings-save admission, and Tauri adaptation are separate one-way layers. The bounded persisted-file owner remains below this inventory threshold. No duplicate applied-config owner. |
| Job migrations/ledger | `jobs/migrations.rs` (337); `jobs/ledger/remote_recovery.rs` (337); `jobs/ledger/row_mapping.rs` (268); `jobs/ledger/records.rs` (262); `jobs/ledger/remote_state.rs` (260); `jobs/ledger.rs` (258); `jobs/ledger/remote_progress.rs` (258); `jobs/ledger/retention.rs` (255) | Each file owns one schema, mapping, recovery, remote-state, progress, or retention surface under the SQLite ledger. |
| Job commands/drain/model projection | `jobs/drain/recovery.rs` (326); `jobs/commands.rs` (330); `jobs/model/status.rs` (258) | Recovery policy, command facade/import-dispatch setup, and status projection delegate to extracted lifecycle, upload, scheduler, ledger, remote, and model owners. The native import dispatcher itself is below 250 lines. |
| Remote preparation/results | `jobs/remote/result.rs` (275); `jobs/remote/preparation.rs` (266) | Result trust validation and source-to-spool preparation are separate artifact boundaries. |
| Live state/runtime | `live/state/owner.rs` (329); `live/runtime/capture_worker.rs` (332); `live/runtime/asr_adapter.rs` (304); `live/runtime/warmup.rs` (273) | State authority, capture processing, model-specific stream adapter, and warmup lifecycle are independent owners. |
| Live actions/hotkeys | `live/shortcut_runtime/dispatcher.rs` (302); `live/shortcut_runtime.rs` (263); `live/hotkey_commands/enrollment.rs` (253); `live/hotkeys/parser.rs` (252) | Fixed-capacity input/action execution, OS registration/startup projection, deliberate enrollment policy, and pure physical-chord parsing are separate owners. The former 466-line mixed shortcut module was decomposed without introducing a generic helper layer. |
| Recording/transcript lifecycle | `live/recordings/transcripts/revision.rs` (317); `file_actions/transcripts.rs` (312); `audio/recording/sidecar_validation.rs` (321); `audio/recording/artifact_admission.rs` (276); `audio/recording/artifact_io.rs` (272); `audio/recording/journal_state.rs` (270); `audio/recording/stream_finalize.rs` (268); `audio/recording/worker.rs` (278); `audio/recording/scan.rs` (252) | Revision, renderer-authorized file action, sidecar trust, admission, I/O, journal state, finalization, worker, and scanning are explicit one-reason modules. |
| Audio contracts/timeline/coordinator | `audio/evidence.rs` (333); `audio/frame/chunk.rs` (287); `audio/frame/chunk/validation.rs` (267); `audio/manifest/envelope.rs` (281); `audio/manifest/window_support.rs` (265); `audio/evidence/wire.rs` (272); `audio/timeline/loss_accumulator.rs` (295); `audio/timeline/track.rs` (257); `audio/coordinator/lifecycle.rs` (258); `audio/capture/callback.rs` (251) | Domain envelope/wire validation, chunk policy, Windows support, bounded loss state, track state, coordinator lifecycle, and real-time callback remain separated around trust/performance boundaries. |
| Media/path boundary | `media_protocol/source.rs` (301); `media_protocol/admission.rs` (273); `media_protocol/server.rs` (271); `paths/legacy_migration.rs` (306); `paths/legacy_migration/secure_tree.rs` (316) | Source leasing, admission, loopback streaming, migration orchestration, and secure tree traversal are distinct. |
| STT model lifecycle | `stt/fallback_model/operation.rs` (304); `stt/model/temp.rs` (286); `stt/fallback_model/progress.rs` (254) | Operation control, temp-artifact policy, and progress projection no longer share a model catch-all. |

### Desktop frontend production

| Owner group | Files (lines) | Inspection outcome |
| --- | --- | --- |
| App composition | `desktop/src/App.tsx` (341) | Composes feature hooks and views; it no longer implements native job, history, settings, polish, or live lifecycles. Reinspect at 350 or if a new feature owner is added. |
| History projections | `lib/history-preview-loader.ts` (339); `components/history/use-history-search.ts` (263); `hooks/use-transcript-history.ts` (262); `lib/history-playback.ts` (254); `lib/playback-admission.ts` (251) | Preview/cache loading, search, compatibility/presentation history, playback projection, and native admission queue are separate. Native catalog/path authority remains below Tauri. |
| Settings/playback/upload | `hooks/use-settings-control.ts` (335); `components/playback/recording-player.tsx` (335); `components/stacked-upload.tsx` (301) | Settings orchestration, media element presentation, and upload/drop rendering have distinct dependencies and no durable authority. |
| Polish/live views | `polish.ts` (309); `components/live/live-overlay-views.tsx` (285) | Polish invoke/save adapter and pure live-island view variants are separate from state/lifecycle owners. |

### Server production

| Owner group | Files (lines) | Inspection outcome |
| --- | --- | --- |
| Job runtime/store | `jobs/runtime.py` (311); `jobs/job_store.py` (307); `jobs/intake_contract.py` (259) | Runtime construction, durable store/locking, and intake schema validation are separate from service coordination. Runtime depends on the stable pool contract rather than its concrete implementation. |
| Batch ASR | `pools/model_lock.py` (311); `pools/batch_asr.py` (303); `pools/batch_asr_worker.py` (258) | Immutable identity/assets, model inference adapter, and worker protocol/process entrypoint have one-way dependencies. |

### Test, contract, fixture, and workflow files

| Test family | Files (lines) | Inspection outcome |
| --- | --- | --- |
| Audio/coordinator/timeline/recording | `audio/coordinator/tests/queue_semantics.rs` (343); `audio/coordinator/tests/lifecycle.rs` (317); `audio/timeline/tests/timeline_semantics.rs` (295); `audio/timeline/tests/concurrency.rs` (273); `audio/recording/tests/worker_failures.rs` (291); `audio/recording/tests/reservation_publication.rs` (270); `audio/recording/tests/publication_security.rs` (261); `audio/manifest/tests/schema_contract.rs` (275); `audio/manifest/tests/windows_core.rs` (258) | Scenario suites are partitioned by invariant/security boundary and share only local fixtures. |
| Connector/job native tests | `server_connector/config/tests/publication.rs` (342); `src-tauri/tests/server_connector.rs` (330); `jobs/commands/tests/retry_security.rs` (322); `jobs/drain/tests/upload.rs` (318); `jobs/commands/tests/cleanup_retention.rs` (312); `jobs/ledger/tests/remote_state.rs` (303); `jobs/ledger/tests.rs` (300); `jobs/commands/tests/authority_admission.rs` (300); `jobs/remote/tests.rs` (277); `jobs/drain/tests/cancellation.rs` (275); `jobs/ledger/tests/lifecycle_retention.rs` (264) | Dedicated configuration, protocol integration, authority, retry, cleanup, upload, ledger, and cancellation matrices. |
| Live/model native tests | `stt/fallback_model/tests.rs` (329); `live/runtime/tests/warmup_finalization.rs` (331); `live/runtime/tests/lifecycle.rs` (276); `stt/nemotron/tests/catalog.rs` (259) | Model and live runtime tests are isolated from implementation owners despite residing under `src`. |
| Frontend unit tests | `tests/unit/history-actions.test.ts` (336); `tests/unit/polish-save-owner.test.ts` (330); `tests/unit/history-catalog-sync.test.ts` (305); `tests/unit/playback-registry.test.ts` (292); `tests/unit/history-storage-prune.test.ts` (270); `tests/unit/workflow-projections.test.ts` (269) | Each suite targets one frontend adapter/projection authority. |
| Native UI/Playwright/WDIO | `tests/wdio/live-overlay.hardware.spec.js` (340); `tests/wdio/live-overlay.spec.js` (324); `tests/wdio/smoke.spec.js` (320); `tests/wdio/live-overlay-window-fixture.js` (311); `tests/e2e/live-overlay.spec.ts` (310); `tests/wdio/phase5-remote-stt.gate.spec.js` (309); `tests/wdio/task-8b-isolation.js` (261) | Hardware-optional, hardware-independent native, browser, Phase 5, and isolation surfaces stay explicit; fixture code is not production. |
| Release contracts | `tests/scripts/release-contract/cache-policy.mjs` (335); `tests/scripts/release-contract/artifact.contract.mjs` (320); `tests/scripts/assert-third-party-provenance.mjs` (297); `tests/scripts/release-contract/cache.contract.mjs` (268) | Cache policy, artifact contract, provenance verification, and cache tests are separate. The stable facade and CLI point down to these owners. |
| Server contract/job/API/infra tests | `server/tests/contract/contract_schema_support.py` (325); `server/tests/jobs/test_service_result_recovery.py` (317); `server/tests/infra/test_server_node_setup.py` (315); `server/tests/jobs/test_service_cancellation_races.py` (314); `server/tests/jobs/test_service_processing.py` (306); `server/tests/jobs/test_service_upload.py` (279); `server/tests/api/api_fixtures.py` (274) | Contract fixture support and scenario suites are partitioned by result recovery, setup policy, cancellation, processing, upload, and HTTP fixture ownership. |
| Hosted CI | `.github/workflows/ci.yml` (342) | Four explicit jobs (frontend, Rust, native WDIO, server) share immutable cache-policy contracts. It is below the threshold and mirrors required hosted check ownership. |

This list covers every hand-written production, test, script, and workflow file
at or above 250 lines at the implementation anchor. Documentation
classification moves preserve history; new canonical/evidence docs and focused
owner modules account for the tracked-file increase from the initial 805-file
review anchor to 829 files. Shared Rust/Python bounded-file readers and the
native-import dispatcher are below 250 lines but are recorded because they own
cross-cutting trust or resource boundaries.

## Decomposition summary

The branch replaced large mixed owners with domain modules in these areas:

- desktop remote job drain, durable ledger, command, and artifact lifecycles;
- server API, job validation/artifacts/store/completion, router, and worker
  boundaries;
- React app/history/settings/polish/live-island feature ownership;
- capture, timeline, coordinator, recording durability, recovery, deletion,
  transcript, playback, and live-runtime ownership;
- connector configuration/core/desktop/health/batch boundaries;
- fallback model download/progress/integrity/Nemotron lifecycle; and
- release artifact/CLI/workflow/process/Git/cache/provenance contracts;
- shared bounded persisted-file I/O without displacing domain validation; and
- fixed-capacity shortcut/import dispatch from OS registration and durable job
  mutation ownership.

The detailed commit order is recorded in [FINDINGS.md](FINDINGS.md). Public
facades were retained only where callers need a stable boundary. No replacement
`context`, `manager`, `common`, or broad helper module remains from the reviewed
extractions.

## External dependency and provenance inventory

| Surface | Locked/declared source of truth |
| --- | --- |
| Frontend | `desktop/package.json` + `desktop/pnpm-lock.yaml`; Node 24, pnpm 11.7.0, React/Tauri/Radix/GSAP and test tooling. |
| Native | `desktop/src-tauri/Cargo.toml` + `Cargo.lock`; Rust 1.96, Tauri 2, reqwest/rustls, Tokio, CPAL, sherpa-onnx, rusqlite, Windows APIs. |
| Portable server | `server/pyproject.toml`; Python `>=3.12,<3.13`, with the service/contract path using the standard-library implementation surface. |
| GPU worker | Digest-pinned `nvcr.io/nvidia/pytorch` base in `server/runtime/asr/Dockerfile`, hash-locked minimal overlay in `requirements.lock`, and immutable model/runtime identities in `server/model-pools.lock.json`. |
| Reviewed source reuse | `third_party/reviewed-sources.json`, `desktop/THIRD_PARTY_NOTICES.md`, server runtime notices/licenses, and the executable provenance contract. |

License/provenance policy is summarized in
[Third-party provenance](../../provenance/THIRD-PARTY.md). Upstream identities
must be verified before code reuse; package lockfiles do not substitute for
source-attribution records.

## Git object and tracked-artifact review

At the anchor, `git count-objects -vH` reported four packs totaling 19.13 MiB,
38 loose objects totaling 36.72 KiB, and no garbage. The largest historical
blobs are application icon source/ICNS assets (1,313,454–1,699,470 bytes), then
`icon.ico` (372,526 bytes), historical `package-lock.json` blobs
(303,111–315,331 bytes), the public app icon (280,685 bytes), and pnpm lockfile
revisions (about 270 KiB).

Findings:

- no private media, transcript, scan result, model weight, build tree, or test
  result is tracked; the one public licensed ASR fixture is the reviewed
  exception described above;
- the current `package-lock.json` is removed and pnpm has one current lockfile;
- icon binaries are legitimate packaging inputs and do not justify a disruptive
  history rewrite; and
- no current Git-bloat remediation is warranted. Continue repository
  housekeeping and ignore enforcement rather than rewriting shared history.
