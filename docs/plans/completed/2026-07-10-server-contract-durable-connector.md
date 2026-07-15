# Server Contract And Durable Connector Implementation Plan

> **Historical record — current authority (2026-07-14):** The server-boundary
> implementation remains valid, but renderer raw-path import commands,
> automatic legacy-path migration, and synthesized `SendInput` delivery
> described in historical tasks are retired. Current client behavior is native
> picker/drop admission plus clipboard-only delivery; use
> [ADR 0013](../../adr/0013-global-hotkey-injection.md) and the
> [client state-machine spec](../../specs/client-state-machine.md) as authority.

> **Implementation status (2026-07-13):** Landed canonical Phase 3 server-boundary record. Tasks 1-7 remain implemented. The stock-NSIS replacement and retained server boundary passed the one-time local/native/server/GB10 implementation gate on exact candidate `c3999b7b685dd668165d54b64d1af61e41adad05`. Implementation head `a721121315c7a4bf5510212196141f17e9b237bd` then passed hosted CI run `29293287930` and disposable-Windows stock NSIS lifecycle run `29293291582`. The unchecked boxes below preserve the original implementation recipe and are not backlog or status evidence.

> **Evidence boundary:** Phase 3 proves the machine-readable contract, health-only service, capability-aware connector, retry behavior, and Rust-owned durable ledger. It does not prove a persistent server service, same-process native UI transition, upload/drain, WSS, authentication, ASR, model pools, or external network exposure.

> **Reader note:** Do not execute this document as an unfinished plan. Use current code, executable tests, and the validation status above as implementation truth.

> **Installer closure amendment (2026-07-13):** The Phase 3 server contract, connector, retry, and durable-ledger implementation remains required and unchanged. Installer lifecycle containment is no longer part of this plan's runtime design: Yap uses Tauri's canonical app-data path and stock NSIS, removes installer-only containment machinery, and runs lifecycle verification only in a disposable Windows environment.

**Goal:** Complete Yap's canonical Phase 3 boundary with a versioned server API/WSS contract, a real desktop reachability connector, and a Rust-owned SQLite job ledger that can safely support later upload and reconnect drain.

**Architecture:** Keep the MVP monorepo. Define machine-readable HTTP and live-event contracts under `server/openapi/`. Run only a small standard-library health service in this phase. Add a focused Rust `server_connector` module for validated settings, bounded health checks, stale-response rejection, and retry state. Add a separate Rust `jobs` module backed by SQLite for job metadata and transitions. React consumes snapshots/events and no longer owns durable queue state. Actual chunk upload, WSS audio streaming, model pools, authentication, and automatic queue drain remain later execution plans.

**Tech Stack:** Python 3.11+ standard library, JSON/OpenAPI 3.1, Tauri 2, Rust 2021, existing `reqwest`, new `rusqlite 0.40.1` with bundled SQLite and default features disabled, existing `serde`/`serde_json`, React 19, TypeScript, Vitest, Playwright, WebdriverIO.

## Global Constraints

- Apply ADR 0014 for the thin-client/server boundary, ADR 0016 for the future auth boundary, ADR 0018 for the Phase 10 repo split, and ADR 0020 for source identity, replay, and durability.
- Keep this monorepo through canonical Phase 9. Do not create `yap-contracts`, Nx, Turborepo, or a second server repo.
- Treat ADR 0002 client runtime details, ADR 0009 team IPC details, and the old Phase 8-12 numbering as historical.
- Health is the only network server behavior implemented here. Job upload and WSS schemas are normative contracts but have no production handler in this plan.
- Do not implement automatic queue drain, model inference, Cohere invocation, diarization, Entra/MSAL, token validation, profile identity, Postgres, Redis, object storage, containers, or public firewall exposure.
- Do not add a Python web framework merely to serve one health route. Re-evaluate a framework when Phase 5 implements concurrent upload/WSS handlers.
- Add SQLite only because durable jobs now need indexed transitions, cancellation, attempts, idempotency, and restart recovery. Do not move WAV/Opus bytes, transcript bodies, credentials, model artifacts, settings, or embeddings into SQLite.
- React and WebView localStorage are projections or migration sources, never native path authorization or job authority.
- The connector accepts HTTPS URLs. Plain HTTP is allowed only for loopback, or for RFC1918 development addresses when `YAP_ALLOW_INSECURE_PRIVATE_SERVER=1` is set in the process environment. The setting is never persisted or exposed as a normal product toggle.
- Before Phase 5 sends audio, a reviewed organization-origin policy, certificate trust path, and ADR 0016 token-audience check must gate the configured server. This health-only plan sends no user audio or transcript content.
- Every request has connect and total timeouts. One configuration generation owns at most one in-flight health request and one retry timer.
- Server ownership fields are absent from client requests until auth exists. Later the server derives `(tenant_id, owner_subject_id)` from the validated token.
- Phase boundary: this plan completes canonical Phase 3 and the durable-ledger prerequisite for Phase 5. It does not claim Phase 5 remote STT is implemented.

## Execution Order

The execution order required the [client audio foundation plan](../archived/2026-07-10-client-audio-foundation.md) first. Its session, track, gap, replay, and capture-commit types are the source contract consumed here. Contract-document work in Tasks 1-2 could be reviewed in parallel, but desktop code had to rebase onto the landed client types instead of defining duplicates.

## Governing Documents

- [ADR 0014: Server-tier compute topology](../../adr/0014-server-tier-compute-topology.md)
- [ADR 0016: Auth and identity bridge](../../adr/0016-auth-identity-bridge.md)
- [ADR 0018: Three-repo topology](../../adr/0018-three-repo-topology.md)
- [ADR 0020: Meeting capture and diarization authority](../../adr/0020-meeting-capture-diarization-authority.md)
- [Client recording state machine](../../specs/client-state-machine.md)
- [Server tier MVP](../../specs/server-tier-mvp.md)
- [Client hardening and storage design](../../archive/historical-designs/2026-07-09-client-hardening-storage-design.md)
- [Source-aware diarization design](../../specs/source-aware-diarization.md)

---

## Starting baseline before implementation (historical)

| Area | Current implementation | Change in this plan |
|------|------------------------|---------------------|
| Server | `server/` has a health function and live/batch route-selection tests, but no process entrypoint or network contract. | Add OpenAPI/live schemas and a loopback-safe health process. |
| Connector | `desktop/src/server.ts` invokes a Rust enum snapshot. Rust never contacts a server. | Add validated settings, async health, retry, cancellation generation, and typed events. |
| Runtime | `RuntimeOrchestrator` has server states and route policy. | Make connector transitions update it; keep it the route authority. |
| Queue | `recording-queue.ts` stores at most 200 server jobs in localStorage. | Migrate once into a Rust-owned SQLite ledger and keep frontend as a typed projection. |
| Batch action | `start_transcribe` validates paths, logs `server_batch_unwired`, then returns `SERVER_UNAVAILABLE`. | Replace fake execution with durable queue/retry commands; no upload until Phase 5. |
| Contract names | Frontend still uses `intent` and `server_processing_cohere`. | Migrate to `sessionMode`, `sessionOrigin`, and model-agnostic `server_processing`. |

## Target Module Shape

```text
server/
  openapi/
    openapi.json
    live-events.schema.json
    examples/
      health.ok.json
      job.accepted.json
      live.partial.json
  src/yap_server/
    api/
      app.py
      health.py
    config/
      settings.py
    schemas/
      contract.py
    workload_router/
  tests/
    contract/
    api/
    workload_router/

desktop/src-tauri/
  migrations/
    0001_job_ledger.sql
  src/
    server_connector/
      mod.rs
      client.rs
      config.rs
      state.rs
    jobs/
      mod.rs
      ledger.rs
      migrations.rs
      model.rs
      commands.rs
```

`desktop/src-tauri/src/lib.rs` registers commands and managed states only. It does not contain connector HTTP logic or SQL.

## Spec Traceability

| Accepted requirement | Plan coverage |
|----------------------|---------------|
| Health, live WSS, batch job, and stable error contracts exist | Task 1 |
| The first server process is private, bounded, and framework-free | Task 2 |
| Settings URL and reachability drive typed connector states | Tasks 3-4 |
| Health-only service cannot advertise unavailable workload paths | Tasks 1, 2, and 4 |
| Pending jobs become Rust-owned before reconnect/drain | Tasks 5-6 |
| WebView queue migration is idempotent and path-safe | Task 6 |
| Monorepo remains canonical and no public app port is opened | Global constraints, Tasks 2 and 7 |
| Upload, WSS runtime, auth, and inference remain unimplemented | Global constraints and final review gate |

---

## Task 1: Freeze The Phase 3 HTTP And Live Event Contracts

**Files:**
- Create: `server/openapi/openapi.json`
- Create: `server/openapi/live-events.schema.json`
- Create: `server/openapi/examples/health.ok.json`
- Create: `server/openapi/examples/job.accepted.json`
- Create: `server/openapi/examples/live.partial.json`
- Create: `server/src/yap_server/schemas/__init__.py`
- Create: `server/src/yap_server/schemas/contract.py`
- Create: `server/tests/contract/__init__.py`
- Create: `server/tests/contract/test_contract.py`
- Modify: `server/README.md`

**HTTP contract:**

| Method | Path | Phase 3 behavior | Later owner |
|--------|------|------------------|-------------|
| `GET` | `/v1/health` | Implemented | Server process |
| `POST` | `/v1/jobs` | Contract only | Phase 5 upload intake |
| `GET` | `/v1/jobs/{jobId}` | Contract only | Phase 5 job status |
| `DELETE` | `/v1/jobs/{jobId}` | Contract only | Phase 5 cancellation |
| `PUT` | `/v1/jobs/{jobId}/chunks/{trackId}/{sequenceStart}-{sequenceEnd}` | Contract only | Phase 5 resumable upload |
| `POST` | `/v1/jobs/{jobId}/commit` | Contract only | Phase 5 upload commit |
| `GET` upgrade | `/v1/live` | Event schema only | Phase 5 WSS streaming |

**Stable error envelope:**

```json
{
  "code": "SERVER_BUSY",
  "message": "Server capacity is temporarily unavailable.",
  "retryable": true,
  "requestId": "req-01J..."
}
```

- [ ] **Step 1: Write failing contract tests before files exist**

Tests must parse JSON with the standard library and assert OpenAPI 3.1, exact paths, operation IDs, required request/response schemas, error envelope fields, replay headers, and live event discriminators.

```python
class ContractTests(unittest.TestCase):
    def test_openapi_exposes_the_phase_3_and_5_boundary(self) -> None: ...
    def test_chunk_contract_separates_replay_key_from_content_hash(self) -> None: ...
    def test_live_events_have_version_and_monotonic_sequence(self) -> None: ...
    def test_examples_conform_to_required_contract_fields(self) -> None: ...
```

Run:

```powershell
$env:PYTHONPATH = "server/src"
python -m unittest server.tests.contract.test_contract -v
```

Expected before implementation: import or file-not-found failures.

- [ ] **Step 2: Define OpenAPI components with model-agnostic names**

Use `RecordingJobStatus` values from `docs/specs/client-state-machine.md`, including `server_processing`, never `server_processing_cohere`. Define `SessionMode`, `SessionOrigin`, `AudioRoute`, `SessionMetadata`, `CaptureTrackDescriptor`, `ChunkReplayKey`, `ContentIdentity`, `AudioGap`, `CaptureManifestReference`, `ResultAuthority`, `ResultStatus`, `TranscriptResultRevision`, `SpeakerResultRevision`, `SpeakerTurn`, `AlignedWord`, `ServerCapabilities`, `RecordingJob`, and `ApiError`. Preserve the client metadata contract for UTC start, optional UTC offset, bounded BCP 47 locale/language hints, explicit ISO alpha-2 country hint, opaque device reference, app/platform version, privacy-policy version, and retention expiry.

Job/chunk requests contain no tenant or owner-subject field. The server adds those only after ADR 0016 token validation exists. `privacyPolicyVersion: "unconfigured"` is valid for local capture artifacts but cannot authorize a team upload once auth/policy gating ships.

Immutable manifest and server enum values use snake_case: `live_capture`, `imported_file`, `local_fallback`, `server_batch`, and `server_live`. JSON field names remain camelCase. The Tauri-to-React `RecordingJobView` is a separate projection that maps those values to `liveCapture`, `importedFile`, `localFallback`, `serverBatch`, and `serverLive`; contract tests must assert both directions explicitly.

Chunk upload uses raw request bytes. Required headers are:

```text
Idempotency-Key: <schema>/<session>/<track>/<sequence-start>/<sequence-end>
X-Yap-Content-SHA256: <64 lowercase hex characters>
X-Yap-Audio-Codec: pcm_s16le
X-Yap-Sample-Rate-Hz: 16000
X-Yap-Channels: 1
Content-Type: application/octet-stream
```

Do not label little-endian PCM as `audio/L16`; that media type implies network byte order. The explicit Yap headers must match the referenced chunk manifest, and a mismatch fails before bytes are accepted.

Same idempotency key plus same hash is replay success. Same key plus a different hash is HTTP 409 `CONTENT_IDENTITY_CONFLICT`.

- [ ] **Step 3: Define live event ordering and reconnect semantics**

All messages include `schemaVersion`, `sessionId`, and monotonically increasing `eventSequence`.

Client events: `session.start`, `audio.chunk`, `audio.gap`, `session.finish`, `session.cancel`, `ping`.

Server events: `session.accepted`, `transcript.partial`, `transcript.final`, `server.backpressure`, `session.error`, `session.finished`, `pong`.

`audio.chunk` references chunk key/hash metadata and carries binary audio in the immediately following WebSocket binary message. The schema must say duplicate final events are idempotent and stale event sequences are ignored.

- [ ] **Step 4: Add dependency-free Python contract dataclasses**

Implement only shapes used by health and tests:

```python
@dataclass(frozen=True, slots=True)
class ServerCapabilities:
    batch_jobs: bool
    live_streaming: bool
    job_status: bool

@dataclass(frozen=True, slots=True)
class HealthView:
    service: str
    status: Literal["ok"]
    api_version: str
    auth: Literal["not_configured", "required"]
    capabilities: ServerCapabilities
```

Use explicit `to_wire()` functions so snake_case Python names map to camelCase wire names without bringing in a validation framework.

- [ ] **Step 5: Run all server tests**

```powershell
$env:PYTHONPATH = "server/src"
python -m unittest discover -s server/tests -p "test_*.py" -v
```

Expected: contract, health, and router tests pass.

- [ ] **Step 6: Commit**

```powershell
git add server/openapi server/src/yap_server/schemas server/tests/contract server/README.md
git commit -m "Define the Yap server contract"
```

---

## Task 2: Add A Small Private Health Service

**Files:**
- Create: `server/src/yap_server/config/__init__.py`
- Create: `server/src/yap_server/config/settings.py`
- Create: `server/src/yap_server/api/app.py`
- Modify: `server/src/yap_server/api/health.py`
- Modify: `server/src/yap_server/api/__init__.py`
- Create: `server/src/yap_server/__main__.py`
- Modify: `server/tests/api/test_health.py`
- Create: `server/tests/api/test_app.py`
- Modify: `server/pyproject.toml`
- Modify: `server/README.md`

**Interfaces:**

```python
@dataclass(frozen=True, slots=True)
class ServerSettings:
    host: str = "127.0.0.1"
    port: int = 18765

def create_server(settings: ServerSettings) -> HTTPServer: ...
def serve(settings: ServerSettings) -> None: ...
```

- [ ] **Step 1: Write network-level tests first**

Start the bounded single-request-at-a-time server on `127.0.0.1:0` in a test thread. Use `urllib.request` to assert:

```python
def test_health_returns_contract_json_and_no_store_headers(self) -> None: ...
def test_unknown_route_returns_the_stable_json_error(self) -> None: ...
def test_non_get_health_method_returns_405(self) -> None: ...
def test_oversized_request_is_rejected_before_body_read(self) -> None: ...
```

Expected before implementation: imports fail.

- [ ] **Step 2: Implement one standard-library handler**

Subclass `BaseHTTPRequestHandler` and use `HTTPServer`. Serve only `GET /v1/health`; all contract-only routes return 501 `NOT_IMPLEMENTED` with `retryable: false`. Suppress default stderr logging and emit one bounded structured log line per request through an injected logger. Do not use `ThreadingHTTPServer`; its unbounded request threads are unnecessary for this health-only phase.

Set `Content-Type: application/json`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, and an exact `Content-Length`. Cap any request body at 1 MiB even though health does not consume one.

- [ ] **Step 3: Make safe binding the default**

Read `YAP_SERVER_HOST` and `YAP_SERVER_PORT`. Default to loopback. Reject wildcard or non-loopback hosts unless `YAP_SERVER_ALLOW_PRIVATE_BIND=1` is set. The setup/runbook can later place TLS/reverse-proxy handling in front; this plan opens no firewall ports.

- [ ] **Step 4: Add the executable entrypoint**

`python -m yap_server` starts the health service and exits cleanly on Ctrl+C. Add no runtime dependency to `pyproject.toml`; only declare the console script:

```toml
[project.scripts]
yap-server = "yap_server.__main__:main"
```

- [ ] **Step 5: Run tests and a loopback smoke**

```powershell
$env:PYTHONPATH = "server/src"
python -m unittest discover -s server/tests -p "test_*.py" -v
```

Start the process in one terminal and query it from another:

```powershell
$env:PYTHONPATH = "server/src"
python -m yap_server
```

```powershell
Invoke-RestMethod http://127.0.0.1:18765/v1/health
```

Expected response fields: `service=yap-server`, `status=ok`, `apiVersion=1`, `auth=not_configured`, and all Phase 5 capabilities set to `false`.

- [ ] **Step 6: Commit**

```powershell
git add server/src/yap_server/config/__init__.py server/src/yap_server/config/settings.py server/src/yap_server/api/app.py server/src/yap_server/api/health.py server/src/yap_server/api/__init__.py server/src/yap_server/__main__.py server/tests/api/test_health.py server/tests/api/test_app.py server/pyproject.toml server/README.md
git commit -m "Serve the private Yap health endpoint"
```

---

## Task 3: Add Validated Desktop Server Settings

**Files:**
- Create: `desktop/src-tauri/src/server_connector/mod.rs`
- Create: `desktop/src-tauri/src/server_connector/config.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src/server.ts`
- Modify: `desktop/src/settings.ts`
- Modify: `desktop/src/components/panels/app-sheets.tsx`
- Test: `desktop/src-tauri/src/server_connector/config.rs`
- Test: `desktop/tests/unit/settings.test.ts`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    pub schema_version: u16,
    pub enabled: bool,
    pub base_url: Option<String>,
}

pub fn validate_base_url(raw: &str, allow_insecure_private: bool) -> Result<String, ConfigError>;
```

- [ ] **Step 1: Add URL and settings persistence tests**

Cover HTTPS, loopback HTTP, private HTTP with and without the environment override, public HTTP rejection, embedded credential rejection, fragments/query rejection, path normalization, malformed JSON recovery, and atomic replacement of a stale partial settings file.

- [ ] **Step 2: Implement focused JSON settings persistence**

Persist `server-settings.json` under `paths::app_data_dir()`. Write `.part`, flush, and atomically replace. Keep only URL and enabled state. Never store bearer tokens, cookies, certificates, or the insecure-development override.

Use `reqwest::Url` for parsing rather than string slicing. Normalize to an origin with no query, fragment, username, password, or trailing `/v1` path.

- [ ] **Step 3: Add typed Tauri commands**

```rust
server_settings() -> Result<ServerSettings, String>
set_server_settings(settings: ServerSettings) -> Result<ServerSettings, String>
```

Restrict both commands to the main window with the existing command guard. Setting changes increment the connector generation and cancel stale work in Task 4.

- [ ] **Step 4: Wire the existing Settings server row**

Use the current settings overlay primitives. Add URL input, enabled toggle, Save, and Test Connection. Keep errors inline and terse. Do not add a mobile layout, nested card, or account/auth UI.

- [ ] **Step 5: Run targeted checks**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml server_connector::config
pnpm --dir desktop test -- settings.test.ts
pnpm --dir desktop build
```

Expected: all tests and build pass.

- [ ] **Step 6: Commit**

```powershell
git add desktop/src-tauri/src/server_connector desktop/src-tauri/src/lib.rs desktop/src/server.ts desktop/src/settings.ts desktop/src/components/panels/app-sheets.tsx desktop/tests/unit/settings.test.ts
git commit -m "Persist validated server settings"
```

---

## Task 4: Implement Reachability, Cancellation, And Retry State

**Files:**
- Create: `desktop/src-tauri/src/server_connector/client.rs`
- Create: `desktop/src-tauri/src/server_connector/state.rs`
- Modify: `desktop/src-tauri/src/server_connector/mod.rs`
- Modify: `desktop/src-tauri/src/runtime/orchestrator.rs`
- Modify: `desktop/src-tauri/src/runtime/state.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src/server.ts`
- Modify: `desktop/src/hooks/use-server-connection.ts`
- Test: `desktop/src-tauri/src/server_connector/client.rs`
- Test: `desktop/src-tauri/src/server_connector/state.rs`
- Test: `desktop/src-tauri/src/runtime/orchestrator.rs`
- Test: `desktop/tests/unit/app-types.test.ts`

**Interfaces:**

```rust
pub struct ServerConnector {
    client: reqwest::Client,
    inner: std::sync::Mutex<ConnectorInner>,
    generation: std::sync::atomic::AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub batch_jobs: bool,
    pub live_streaming: bool,
    pub job_status: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConnectionSnapshot {
    pub state: ServerConnectorState,
    pub checked_at_ms: Option<u64>,
    pub retry_at_ms: Option<u64>,
    pub api_version: Option<String>,
    pub capabilities: ServerCapabilities,
    pub error_code: Option<String>,
}
```

- [ ] **Step 1: Write state-projection and stale-response tests**

Use a local Rust `TcpListener` fixture to simulate healthy JSON, unsupported API version, malformed JSON, 401, 500, connection refusal, delayed response, and response arriving after config generation changes.

Add tests proving only the newest generation can update state, disabled/not-set never schedule retries, one generation owns at most one health request, and retry delay follows 1, 2, 4, 8, 15, 30 seconds with an injected zero-jitter source in tests. Production adds up to 20 percent positive jitter derived from process/time entropy without adding a random-number dependency, preventing many clients from retrying in lockstep.

- [ ] **Step 2: Build one bounded `reqwest::Client`**

Use existing `reqwest` with a 2-second connect timeout, 3-second total timeout, redirect policy `none`, bounded response size of 64 KiB, and no cookie store. Validate final URL before every request.

- [ ] **Step 3: Implement explicit connector transitions**

```text
not_set -> connecting -> ready
disabled -> disabled
connecting -> offline         timeout/refused/5xx/malformed
connecting -> sign_in_required 401/403 or health auth=required
offline -> retrying -> connecting
ready -> retrying             failed explicit refresh or future socket loss
any -> not_set/disabled       settings change
```

An unsupported `apiVersion` becomes `offline` with `INCOMPATIBLE_API_VERSION`, advertises no capabilities, and does not schedule automatic retry because a client/server upgrade is required. Malformed capability fields fail closed in the same way.

Update `RuntimeOrchestrator::set_server` on each accepted transition and store the advertised capabilities beside connector state. A retry task captures its generation and exits before mutation if the generation changed.

- [ ] **Step 4: Replace the fake status command**

Commands:

```rust
server_connection_status() -> Result<ServerConnectionSnapshot, String>
refresh_server_connection() -> Result<ServerConnectionSnapshot, String>
```

Emit `server-connection` after accepted changes. The frontend hook loads one snapshot, listens for events, and never runs its own retry timer.

- [ ] **Step 5: Preserve offline fallback policy**

Live routing may use the server only when connector state is `ready` and `live_streaming` is advertised; otherwise it uses local fallback when available. Imported recordings remain server jobs and require `batch_jobs`; health-only readiness leaves them queued or blocked and never routes them through local Nemotron. This task changes state truth, not remote execution.

- [ ] **Step 6: Run connector and frontend tests**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml server_connector
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml runtime::orchestrator
pnpm --dir desktop test -- app-types.test.ts
pnpm --dir desktop build
```

Expected: connector tests pass without an external server.

- [ ] **Step 7: Commit**

```powershell
git add desktop/src-tauri/src/server_connector desktop/src-tauri/src/runtime desktop/src-tauri/src/lib.rs desktop/src/server.ts desktop/src/hooks/use-server-connection.ts desktop/tests/unit/app-types.test.ts
git commit -m "Connect desktop server reachability"
```

---

## Task 5: Add The Rust-Owned SQLite Job Ledger

**Files:**
- Modify: `desktop/src-tauri/Cargo.toml`
- Modify: `desktop/src-tauri/Cargo.lock`
- Create: `desktop/src-tauri/migrations/0001_job_ledger.sql`
- Create: `desktop/src-tauri/src/jobs/mod.rs`
- Create: `desktop/src-tauri/src/jobs/model.rs`
- Create: `desktop/src-tauri/src/jobs/migrations.rs`
- Create: `desktop/src-tauri/src/jobs/ledger.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Test: `desktop/src-tauri/src/jobs/migrations.rs`
- Test: `desktop/src-tauri/src/jobs/ledger.rs`

- [ ] **Step 1: Add the pinned SQLite dependency**

Run from `desktop/src-tauri`:

```powershell
cargo add rusqlite@0.40.1 --no-default-features --features bundled
```

Commit the resulting lockfile only with the working ledger task, not as a dependency-only commit.

- [ ] **Step 2: Write migration and repository tests first**

Use in-memory SQLite for state tests and a temporary file for restart tests. Cover schema version, foreign keys, WAL mode on file databases, busy timeout, rollback on migration failure, idempotent reopen, invalid enum rows, restart recovery, concurrent readers, and transaction rollback.

- [ ] **Step 3: Create a constrained schema**

```sql
PRAGMA foreign_keys = ON;

CREATE TABLE recording_jobs (
  job_id TEXT PRIMARY KEY,
  session_mode TEXT NOT NULL CHECK (session_mode IN ('dictation', 'meeting')),
  session_origin TEXT NOT NULL CHECK (session_origin IN ('live_capture', 'imported_file')),
  source_path TEXT,
  source_ownership TEXT NOT NULL DEFAULT 'external' CHECK (source_ownership IN ('external', 'yap_spool')),
  output_path TEXT,
  display_name TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN (
    'accepted', 'preflighting', 'blocked_setup_required',
    'blocked_server_unavailable', 'blocked_sign_in_required',
    'queued_local_fallback', 'queued_server', 'preprocessing',
    'uploading', 'server_processing', 'local_transcribing', 'saving',
    'diarization_queued', 'diarization_running', 'complete', 'partial',
    'failed', 'cancelled'
  )),
  route TEXT CHECK (route IS NULL OR route IN ('local_fallback', 'server_batch', 'server_live')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  next_attempt_at_ms INTEGER,
  cancellation_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancellation_requested IN (0, 1)),
  capture_commit_path TEXT,
  capture_manifest_sha256 TEXT,
  error_code TEXT,
  error_message TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  expires_at_ms INTEGER,
  CHECK (session_origin = 'live_capture' OR source_path IS NOT NULL)
);

CREATE INDEX recording_jobs_status_retry_idx
  ON recording_jobs(status, next_attempt_at_ms, created_at_ms);

CREATE TABLE job_chunks (
  job_id TEXT NOT NULL REFERENCES recording_jobs(job_id) ON DELETE CASCADE,
  owner_namespace TEXT NOT NULL,
  session_id TEXT NOT NULL,
  track_id TEXT NOT NULL,
  sequence_start INTEGER NOT NULL,
  sequence_end INTEGER NOT NULL,
  content_sha256 TEXT NOT NULL,
  artifact_path TEXT NOT NULL,
  upload_offset INTEGER NOT NULL DEFAULT 0,
  acknowledged_object_id TEXT,
  acknowledged_at_ms INTEGER,
  PRIMARY KEY (job_id, track_id, sequence_start, sequence_end),
  CHECK (sequence_end >= sequence_start),
  CHECK (upload_offset >= 0)
);

PRAGMA user_version = 1;
```

SQLite stores paths only after Rust validation. It stores no file bytes. Reject timestamps, sequence values, offsets, or counters that cannot fit SQLite's signed 64-bit integer range before opening a transaction. Phase 3 imports use `source_ownership='external'`; expiry or cancellation must never delete that user-owned source. A later Phase 5 spool copy uses `yap_spool` and may be deleted only through its Rust-owned artifact registry.

- [ ] **Step 4: Implement typed transitions**

`RecordingJobStatus`, `SessionMode`, `SessionOrigin`, and `RecordingRoute` are Rust enums with strict `as_db`/`from_db` mappings. Invalid stored values return a corruption error and do not become an invented UI state.

Do not serialize the database structs directly to React. Add an explicit `RecordingJobView::from_record` projection so persisted snake_case values map to the camelCase TypeScript contract without changing the immutable/server wire format.

Allowed transition checks live in one pure function. Cancellation is allowed only from accepted, blocked, or queued states. Retry increments `attempt_count` transactionally and returns to `preflighting`; it cannot jump directly to uploading.

- [ ] **Step 5: Implement ledger durability settings**

Open `jobs.sqlite3` under `paths::app_data_dir()`. Use WAL, `synchronous=FULL`, foreign keys, and a five-second busy timeout. Run migrations in `BEGIN IMMEDIATE`. Keep one connection behind a `Mutex`; never hold it while performing HTTP, file hashing, or Tauri event emission.

- [ ] **Step 6: Run database tests and audit the file contents**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml jobs::
cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
```

In the restart fixture, query every table and assert that WAV bytes, transcript text, credentials, and embedding-shaped arrays are absent.

- [ ] **Step 7: Commit**

```powershell
git add desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock desktop/src-tauri/migrations desktop/src-tauri/src/jobs desktop/src-tauri/src/lib.rs
git commit -m "Add the durable recording job ledger"
```

---

## Task 6: Move Queue And Job Lifecycle Ownership From React To Rust

**Files:**
- Create: `desktop/src-tauri/src/jobs/commands.rs`
- Modify: `desktop/src-tauri/src/jobs/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/src/batch_recordings.rs`
- Modify: `desktop/src/lib/app-types.ts`
- Modify: `desktop/src/lib/history-utils.ts`
- Modify: `desktop/src/lib/playback-registry.ts`
- Modify: `desktop/src/lib/setup-model-state.ts`
- Modify: `desktop/src/recording-queue.ts`
- Modify: `desktop/src/stt.ts`
- Modify: `desktop/src/App.tsx`
- Modify: `desktop/src/hooks/use-recording-selection.ts`
- Modify: `desktop/src/hooks/use-registered-playback.ts`
- Modify: `desktop/src/hooks/use-transcript-file-actions.ts`
- Modify: `desktop/src/components/stacked-upload.tsx`
- Modify: `desktop/src/components/transcript-review-dialog.tsx`
- Modify: `desktop/src/components/panels/polish-panel.tsx`
- Modify: `desktop/src/components/panels/transcript-panel.tsx`
- Modify: `desktop/src/components/panels/queue-panel.tsx`
- Test: `desktop/src-tauri/src/jobs/commands.rs`
- Test: `desktop/tests/unit/app-types.test.ts`
- Test: `desktop/tests/unit/history-utils.test.ts`
- Test: `desktop/tests/unit/playback-registry.test.ts`
- Test: `desktop/tests/unit/recording-queue.test.ts`
- Test: `desktop/tests/unit/setup-model-state.test.ts`

**Rust command surface:**

```rust
recording_jobs_snapshot() -> Result<Vec<RecordingJobView>, JobCommandError>
recording_jobs_create_imports(paths: Vec<String>) -> Result<Vec<RecordingJobView>, JobCommandError>
recording_jobs_import_legacy(payload: LegacyQueueImport) -> Result<LegacyImportResult, JobCommandError>
recording_job_cancel(job_id: String) -> Result<RecordingJobView, JobCommandError>
recording_job_retry(job_id: String) -> Result<RecordingJobView, JobCommandError>
```

- [ ] **Step 1: Write command and projection tests first**

Cover path validation and native allowlisting, duplicate file import, missing file, moved/reparse-point file after restart, 200-job product bound, string ID stability, cancellation/retry legality, snapshot ordering, seven-day pending-job expiry, and event emission after commit rather than before.

- [ ] **Step 2: Migrate TypeScript vocabulary exactly once**

Change:

```ts
type RecordingJobView = {
  id: string;
  sourcePath?: string;
  playbackPath?: string;
  outputPath?: string;
  name: string;
  sessionMode: "dictation" | "meeting";
  sessionOrigin: "liveCapture" | "importedFile";
  status: RecordingJobStatus;
  route?: "localFallback" | "serverBatch" | "serverLive";
  pipeline: RecordingPipelineState;
};
```

`playbackPath` is a Rust-authorized ephemeral projection and is never stored as authority in SQLite. Rename `server_processing_cohere` to `server_processing` in types, labels, tests, and components. Do not retain a second alias after migration code has converted old stored values.

History-only projections use a stable `history:<normalized-output-path>` string ID and remain outside the job ledger. Imported queue jobs receive Rust-minted IDs. Remove numeric ID allocation and the React `unblockFallbackReadyQueue` mutation; setup changes ask Rust to re-preflight eligible jobs.

- [ ] **Step 3: Make `recording-queue.ts` an async bridge plus one-time importer**

Keep the `yap.recordingQueue.v1` parser only for migration. On the first DB-capable launch:

1. Parse and bound legacy localStorage.
2. Restore/authorize paths through Rust.
3. Invoke `recording_jobs_import_legacy({ schemaVersion: 1, jobs })`.
4. Remove the localStorage key only after Rust commits and acknowledges every accepted/duplicate/rejected row.
5. Replaying after a crash is idempotent by deterministic `legacy-<numeric-id>-<path-hash-prefix>` job IDs.

Migration failure is visible and retryable. App startup must not drain or overwrite the queue while migration is unresolved.

- [ ] **Step 4: Replace App-owned queue mutation**

Add a `useRecordingJobs` hook that loads `recording_jobs_snapshot`, listens to `recording-jobs-changed`, and exposes create/cancel/retry command wrappers. Remove `queueRef`, `nextRecordingId`, `writeRecordingQueue`, and the path-based `transcribeItems` mutation loop from `App.tsx`.

The Run action performs preflight/retry only. If connector state is not ready, jobs stay `queued_server` or `blocked_server_unavailable`. It never invokes local Nemotron for imported files.

On restart, treat the ledger as durable provenance but not proof that the filesystem is unchanged. Canonicalize and revalidate each external source before restoring it to the native path allowlist. Keep a missing or unsafe source as a visible failed job with `SOURCE_MISSING` or `SOURCE_UNSAFE`; never silently delete the row or pass the stale path to open, reveal, playback, hashing, or future upload.

- [ ] **Step 5: Retire the misleading native command**

Remove `start_transcribe(paths)` after all callers are gone. Keep local fallback file testing behind a test-only Rust entrypoint or the existing parity harness, not a product command.

- [ ] **Step 6: Run migration, UI, and Rust tests**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml jobs::commands
pnpm --dir desktop test -- app-types.test.ts recording-queue.test.ts setup-model-state.test.ts
pnpm --dir desktop build
```

Expected: tests pass, no product code references `intent`, `server_processing_cohere`, or `start_transcribe`.

- [ ] **Step 7: Commit**

```powershell
git add desktop/src-tauri/src/jobs desktop/src-tauri/src/lib.rs desktop/src-tauri/src/batch_recordings.rs desktop/src/App.tsx desktop/src/lib/app-types.ts desktop/src/lib/history-utils.ts desktop/src/lib/playback-registry.ts desktop/src/lib/setup-model-state.ts desktop/src/recording-queue.ts desktop/src/stt.ts desktop/src/hooks/use-recording-selection.ts desktop/src/hooks/use-registered-playback.ts desktop/src/hooks/use-transcript-file-actions.ts desktop/src/components/stacked-upload.tsx desktop/src/components/transcript-review-dialog.tsx desktop/src/components/panels/polish-panel.tsx desktop/src/components/panels/transcript-panel.tsx desktop/src/components/panels/queue-panel.tsx desktop/tests/unit/app-types.test.ts desktop/tests/unit/history-utils.test.ts desktop/tests/unit/playback-registry.test.ts desktop/tests/unit/recording-queue.test.ts desktop/tests/unit/setup-model-state.test.ts
git commit -m "Move recording jobs into Rust"
```

---

## Task 7: Verify Offline, Restart, And Contract Boundaries

**Files:**
- Create: `desktop/src-tauri/tests/server_connector.rs`
- Create: `desktop/src-tauri/tests/job_ledger.rs`
- Modify: `desktop/tests/e2e/app.spec.ts`
- Modify: `desktop/tests/wdio/smoke.spec.js`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/specs/client-state-machine.md`
- Modify: `docs/specs/server-tier-mvp.md`
- Modify: `docs/adr/0014-server-tier-compute-topology.md`
- Modify: former combined architecture document, now `docs/archive/historical-designs/2026-07-15-voice-os-architecture-pre-checkpoint.md`

- [ ] **Step 1: Add restart and partial-migration integration tests**

Create a temporary app-data directory, import jobs, close the ledger, reopen it, and assert IDs/status/attempts persist. Interrupt legacy migration before localStorage deletion, replay it, and assert no duplicate rows.

- [ ] **Step 2: Test connector failure modes end to end**

Run against the Python health process and Rust failure fixtures. Assert healthy, refused, timeout, malformed response, auth required, disabled, config change during request, and retry cancellation.

- [ ] **Step 3: Test product routing offline**

Playwright: with server unset/offline, imported recordings remain visible and queued; Run does not claim transcription started; local live fallback setup remains available.

WDIO: restart the desktop and assert the queued imported recording returns from Rust without relying on WebView localStorage. Do not require actual upload or ASR.

- [ ] **Step 4: Add CI contract coverage**

Keep the existing server unittest job. Add a step that starts `python -m yap_server` on loopback, polls health for at most ten seconds, runs the Rust connector integration test against it, and always terminates the process. Do not bind a public interface.

Add `actions/setup-python@v6` to the Rust job, then use this PowerShell step from the repository root:

```yaml
- name: contract connector integration
  working-directory: ${{ github.workspace }}
  shell: pwsh
  run: |
    $env:PYTHONPATH = "$PWD\server\src"
    $env:YAP_SERVER_HOST = "127.0.0.1"
    $env:YAP_SERVER_PORT = "18765"
    $server = Start-Process python -ArgumentList "-m", "yap_server" -PassThru -WindowStyle Hidden
    try {
      $ready = $false
      for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try {
          Invoke-RestMethod "http://127.0.0.1:18765/v1/health" | Out-Null
          $ready = $true
          break
        } catch {
          Start-Sleep -Milliseconds 500
        }
      }
      if (-not $ready) { throw "Yap server health did not become ready" }
      $env:YAP_TEST_SERVER_URL = "http://127.0.0.1:18765"
      cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml --test server_connector
      if ($LASTEXITCODE -ne 0) { throw "server connector integration failed" }
    } finally {
      Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
```

- [ ] **Step 5: Run the full verification matrix**

```powershell
$env:PYTHONPATH = "server/src"
python -m unittest discover -s server/tests -p "test_*.py" -v
pnpm --dir desktop test
pnpm --dir desktop build
pnpm --dir desktop test:e2e
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
```

On the native Windows test machine:

```powershell
pnpm --dir desktop test:desktop:all
```

Expected: all checks pass. No test requires a DGX, internet access, credentials, model pool, or corporate network.

- [ ] **Step 6: Update status language precisely**

Mark Phase 3 contract, health, connector state, and durable ledger implemented. Keep WSS transport, chunk upload, queue drain, server ASR, auth, model pools, and server processing unchecked. Change the client-state spec ownership note so React is a projection and Rust is authoritative.

- [ ] **Step 7: Commit**

```powershell
git add desktop/src-tauri/tests desktop/tests/e2e/app.spec.ts desktop/tests/wdio/smoke.spec.js .github/workflows/ci.yml docs/specs/client-state-machine.md docs/specs/server-tier-mvp.md docs/adr/0014-server-tier-compute-topology.md docs/archive/historical-designs/2026-07-15-voice-os-architecture-pre-checkpoint.md
git commit -m "Verify the durable server boundary"
```

---

## Final Review Gate

- [ ] OpenAPI and live-event examples are machine-readable and contract-tested.
- [ ] The Python process exposes health only and defaults to loopback.
- [ ] URL validation rejects credentials, redirects, public plain HTTP, query, and fragment input.
- [ ] Connector requests have connect/total timeouts and stale generations cannot mutate state.
- [ ] Health-only readiness cannot satisfy live-streaming or batch-job route checks.
- [ ] Retry has one owner, is bounded, and stops on disable/config change.
- [ ] SQLite contains job/replay metadata only and survives restart.
- [ ] Legacy localStorage migration is bounded, idempotent, and deletes its source key only after acknowledgement.
- [ ] Rust owns job transitions; React projects snapshots/events.
- [ ] Imported recordings never run through local Nemotron.
- [ ] No automatic upload/drain, WSS runtime, auth, model inference, database server, or firewall exposure landed.
- [ ] Docs distinguish implemented Phase 3 behavior from contract-only Phase 5 behavior.
