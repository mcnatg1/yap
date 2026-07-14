# Yap

Yap is a staged monorepo for the MVP: one desktop app, one staged server tier, and the docs that keep the architecture honest.

The desktop is a Tauri app with a local Nemotron live fallback. Larger
recording transcription belongs on the GB-class server path. Phase 4 now has a
real isolated Cohere reference worker on that node; the desktop upload/drain and
persistent private service that connect it remain later gates. The long-term
split is still `yap-desktop`, `yap-server`, and `yap-knowledge` (ADR 0018), but
staying in this repo through MVP avoids cross-repo churn while the server
contract is still moving.

## Current Posture

- Local/live fallback: Nemotron 3.5 ASR Streaming 0.6B INT8 through in-process `sherpa-onnx`.
- Product surface: installed Tauri desktop app only. Do not add phone/mobile-specific
  layouts; support normal and narrow desktop windows.
- Local fallback install: explicit setup/settings action; runtime never silently downloads models.
- Client workflow: Rust-owned durable imported-job state, typed React projections, connector health/retry state, and reserved future preprocessing/diarization stages.
- Batch/large recordings: locked GB10 Cohere reference pool exists; authorized
  desktop upload/drain and production service integration remain pending.
- Offline without a suitable server/GPU path: queue or block instead of producing low-confidence
  official-looking transcripts.
- Nemotron native punctuation stays enabled; no extra local punctuation model is installed.
- Editor-specific config is not tracked. Use whichever editor you want.
- MVP repo posture: staged monorepo, no Nx/Turborepo, no separate `yap-contracts` yet.

Repo-owned Windows automation now requires PowerShell Core 7.4 or newer, selects `pwsh.exe` explicitly, and verifies that floor in every Windows CI job. A hash-pinned PowerShell 7.4.17 compatibility lane parses every tracked script at the supported minor-version floor. Yap stores runtime data in Tauri's canonical app-data directory (`%APPDATA%\com.mcnatg1.yap` on Windows), serializes and hash-verifies a staged transition of recognized runtime entries from the former `%LOCALAPPDATA%\Yap` location without moving installer files or overwriting conflicts, uses Tauri's stock NSIS behavior, and confines installer lifecycle execution to disposable Windows environments. The canonical Phase 3 server boundary remains implemented: machine-readable contracts, loopback capability health, connector state/retry, and the durable desktop job ledger. Phase 4 adds a separate bounded router/pool and transient isolated Cohere GPU worker. Upload drain, WSS, authenticated server sessions, persistent deployment, and client-connected server processing remain later product work.

The Phase 3 implementation gate is pinned to immutable release `c3999b7b685dd668165d54b64d1af61e41adad05`: the GB10 ARM64/Python 3.12 server suite passed 50/50, transient loopback health reached the command-line production connector as `Ready`, a separate refused-tunnel run projected `Retrying`, and teardown left no Yap process or port-18765 listener. The deployment archive SHA-256 is `be7f43d757821c3e74d0ae2809599f5a84b369115d24afce42fe6687b1bf12e1`. Implementation head `a721121315c7a4bf5510212196141f17e9b237bd` then passed hosted CI run `29293287930`, including the checksum-pinned RustSec audit, and stock NSIS lifecycle run `29293291582` on a disposable `windows-2025` runner. No persistent service, external application bind, or firewall change was introduced.

The Phase 4 executable tree is pinned to exact candidate `309a2d427707e3483b2649f13940bd48dfaee836`. Its one-time local/native/server/GB10 gate passed: the transient ARM64 image `sha256:8b98372d980b3d3ae3cb8bb5cc1498141d161d15157cbd6114339e7a31b8ddff` ran the locked Cohere revision on an NVIDIA GB10 with Python 3.12.3, NVIDIA Torch `2.13.0a0+8145d630e8.nv26.06`, CUDA 13.3, BF16, and WER `0.0` against the `0.12` ceiling. Result SHA-256 `1a2850ad767489e00f6a496a46f95384d0d14b4a609d537a27a1304b80cfbbf0` is bound by evidence SHA-256 `3157efc6845d3c03e05e22a5ad5d0a2e216de5ae26ae990501586a2dfa45312b`; before/after listener, firewall-policy, and service observations matched, and teardown left no Phase 4 container or worker. This proves the isolated reference slice, not the deferred upload/drain, authenticated service, long-recording capacity, or production deployment.

## Repository Layout

```text
.
|-- README.md                         This map.
|-- PRODUCT.md                        Product intent and UX boundaries.
|-- DESIGN.md                         UI/design direction.
|-- docs/
|   |-- VOICE-OS-ARCHITECTURE.md      Current architecture narrative.
|   |-- adr/                          Architecture decisions.
|   |-- specs/                        Phase specs and testing notes.
|   |-- runbooks/                     Operational setup notes.
|   `-- superpowers/plans/            Active and historical implementation plans.
|-- infra/
|   `-- yap-server-node/              GB-class node setup script and env example.
|-- server/
|   `-- README.md                     MVP staging plus the private Phase 4 ASR reference runtime.
`-- desktop/
    |-- README.md                     Desktop-only quick commands.
    |-- package.json                  pnpm scripts and frontend deps.
    |-- tests/                        Frontend unit tests, fixtures, E2E, WDIO, scripts, results.
    |   |-- unit/                     Vitest specs for TS/React helpers.
    |   |-- fixtures/                 Deterministic test data generators.
    |   |-- e2e/                      Playwright specs and snapshots.
    |   |-- wdio/                     WebdriverIO desktop smoke specs and capabilities.
    |   |-- scripts/                  Test runner helper scripts.
    |   `-- results/                  Ignored test traces, screenshots, and reports.
    |-- vite.config.ts                Vite/Tauri dev server config.
    |-- src/                          React app.
    |   |-- App.tsx                   Main app state and screen composition.
    |   |-- live.ts                   Tauri live-session event/invoke client.
    |   |-- recording-queue.ts        Durable recording-job projection client and legacy recovery notice.
    |   |-- settings.ts               Tauri fallback/setup event/invoke client.
    |   |-- history.ts                localStorage transcript history.
    |   |-- polish.ts                 Text polish client.
    |   |-- components/app/           App chrome, sidebar, status widgets.
    |   |-- components/panels/        Main workspace panels.
    |   |-- components/ui/            UI primitives actually used by the app.
    |   |-- hooks/                    Small React hooks.
    |   `-- lib/                      Shared TS helpers/types.
    `-- src-tauri/
        |-- Cargo.toml                Rust/Tauri deps.
        |-- tauri.conf.json           Window and stock bundle config.
        |-- examples/nemotron_profile.rs Local live runtime profiler.
        |-- tests/parity.rs           Mock verbose-json parity contract.
        |-- tests/job_ledger.rs       File-backed restart, bounds, and transition proof.
        |-- tests/server_connector.rs Health/failure/retry and Python contract integration proof.
        `-- src/
            |-- app.rs                App/tray/window lifecycle.
            |-- lib.rs                Tauri wiring and command registration.
            |-- commands/             Native invoke handlers, including media admission.
            |-- audio/                Capture, framing, timeline, bounded sinks, recording, evidence/results.
            |-- live/                 Live session, hotkeys, overlay, injection, runtime, history/recovery.
            |-- jobs/                 Rust-owned SQLite recording-job ledger, transitions, and command boundary.
            |-- runtime/              Route/orchestrator state and connector projection.
            |-- server_connector/     Validated settings, bounded health client, retry/cancellation state.
            `-- stt/
                |-- dispatch.rs       Local fallback runtime state only.
                |-- error.rs          Stable STT error codes/messages.
                |-- model.rs          Shared model cache/download/verification helpers.
                |-- nemotron.rs       Pinned sherpa-onnx Nemotron model bundle.
                |-- parity.rs         Small WER/timestamp helpers for tests.
                `-- settings.rs       Local fallback and compute-target settings.
```

## Current Local Fallback Flow

Official large recordings use or wait for the `serverBatch` contract path. The
local path in this branch is the explicit Nemotron INT8 fallback for live/offline
work, plus setup/install/remove controls for the pinned sherpa model artifacts.

1. The React UI calls live/setup commands through `desktop/src/live.ts` and `desktop/src/settings.ts`.
2. Tauri owns mic capture, hotkey state, the live overlay window, and the local fallback runtime.
3. `stt::nemotron` resolves/downloads the pinned Nemotron INT8 artifact set; the live worker keeps one in-process sherpa recognizer warm.
4. Larger recording jobs are committed to the Rust-owned SQLite ledger and
   remain queued until Phase 5 connects them to the isolated server batch pool,
   instead of silently producing official-looking local transcripts.

Imported recording jobs use Rust-minted string IDs and a SQLite ledger as
authority. React renders typed snapshots/events, while the native picker and
native OS-drop path establish source authority without accepting renderer raw
pathnames. The old `yap.recordingQueue.v1` localStorage value is never migrated
or executed automatically; it remains visible only as a recovery reference
until the user explicitly discards it and re-adds trusted files through the
native picker. The Phase 3 connector validates configured server origins and
capability health, and native code requires explicit approval of the exact
origin before opening health sockets or retries. It does not upload, drain, or
transcribe queued imports. The Phase 4 worker is server-internal and does not
change those advertised capabilities.

## Development

Use Node 24 LTS. This repo has a root `.node-version`, and `desktop/package.json` rejects
Node 25+ because the Tauri/WebdriverIO desktop smoke path is sensitive to Node runtime drift.

```powershell
cd C:\dev\cohere-transcribe-local\desktop
node -v  # should be v24.x
pnpm install
pnpm test
pnpm build
pnpm test:e2e
cargo test --locked --manifest-path .\src-tauri\Cargo.toml
pnpm tauri dev
```

Desktop automation checks:

```powershell
cd C:\dev\cohere-transcribe-local\desktop
pnpm test:e2e:update       # refresh Playwright visual snapshots intentionally
pnpm test:desktop:build    # builds the WDIO-enabled debug Tauri binary
pnpm test:desktop          # runs the WebdriverIO/Tauri smoke test
```

`pnpm test:desktop` expects `src-tauri\target\debug\yap-desktop.exe` unless `APP_BINARY` points at
another build. The WDIO hooks are only compiled when `test:desktop:build` uses the `wdio` feature
and `src-tauri\tauri.wdio.conf.json`.

Useful narrow checks:

```powershell
cd C:\dev\cohere-transcribe-local
git diff --check
Get-Content docs\runbooks\repo-housekeeping.md
```

Local live profiling:

```powershell
cd C:\dev\cohere-transcribe-local
cargo run --release --manifest-path .\desktop\src-tauri\Cargo.toml --example nemotron_profile -- <clip.wav> [reference.txt]
```

The profiler exercises the same in-process Nemotron `LiveStreamEngine` as the app and reports load time, decode time, real-time factor, first-text latency, optional WER, and transcript output.

## MVP Monorepo Rule

Keep code here until the first server path is real enough to split:

- `desktop/` owns the installed client and local fallback.
- `server/` stages the `yap-server` contract and first tested server-tier code.
- `infra/yap-server-node/` owns host/bootstrap setup for GB-class server nodes.
- `docs/` stays authoritative while the architecture is still moving.

Do not add monorepo tooling until two packages actually need shared commands or dependency graphing. `pnpm -C desktop ...` and direct server commands are enough for now.
Use [docs/runbooks/repo-housekeeping.md](docs/runbooks/repo-housekeeping.md) for naming rules and the current tech-debt ledger.

## Local Files

These are intentionally local and ignored:

- `desktop/node_modules/`
- `desktop/dist/`
- `desktop/src-tauri/target/`
- `.tools/`, `.xwin-cache/`, `.xwin-sdk/`
- `.impeccable/`
- editor folders
- logs and local smoke clips

## Not In This Branch

- No Python STT fallback runtime.
- No local Cohere batch default.
- No runtime STT model/backend selector.
- No command palette or generated UI drawer.
- No editor-specific project config.
- No production upload drain, WSS transport, authenticated server session,
  persistent model service, or client-connected server inference path yet; the
  implemented connector is limited to validated settings and
  capability-health state/retry, while the isolated Phase 4 pool is gated
  separately on the private node.
