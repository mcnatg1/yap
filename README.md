# Yap

Yap is a staged monorepo for the MVP: one desktop app, one staged server tier, and the docs that keep the architecture honest.

The desktop is a Tauri app with a local Nemotron live fallback. Larger recording transcription belongs on the GB-class server path once the staged `yap-server` skeleton becomes a deployable private service with real workers. The long-term split is still `yap-desktop`, `yap-server`, and `yap-knowledge` (ADR 0018), but staying in this repo through MVP avoids cross-repo churn while the server contract is still moving.

## Current Posture

- Local/live fallback: Nemotron 3.5 ASR Streaming 0.6B INT8 through in-process `sherpa-onnx`.
- Product surface: installed Tauri desktop app only. Do not add phone/mobile-specific
  layouts; support normal and narrow desktop windows.
- Local fallback install: explicit setup/settings action; runtime never silently downloads models.
- Client workflow: typed recording-job state for setup, local fallback, future server routing, preprocessing, and diarization.
- Batch/large recordings: future GB-class server Cohere path.
- Offline without a suitable server/GPU path: queue or block instead of producing low-confidence
  official-looking transcripts.
- Nemotron native punctuation stays enabled; no extra local punctuation model is installed.
- Editor-specific config is not tracked. Use whichever editor you want.
- MVP repo posture: staged monorepo, no Nx/Turborepo, no separate `yap-contracts` yet.

Repo-owned Windows automation now requires PowerShell Core 7.4 or newer, selects `pwsh.exe` explicitly, and verifies that floor in every Windows CI job. A hash-pinned PowerShell 7.4.17 compatibility lane parses every tracked script and runs the focused native-process suite at the supported minor-version floor. This tooling boundary does not change the product architecture order: the next **product** implementation plan remains the canonical Phase 3 server contract and durable connector.

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
|   `-- README.md                     MVP staging area for future yap-server work.
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
        |-- tauri.conf.json           Window and bundle config.
        |-- nsis-hooks.nsh            Windows installer/uninstaller policy hooks.
        |-- examples/nemotron_profile.rs Local live runtime profiler.
        |-- tests/parity.rs           Mock verbose-json parity contract.
        `-- src/
            |-- app.rs                App/tray/window lifecycle.
            |-- lib.rs                Tauri wiring and command registration.
            |-- commands/             Native invoke handlers, including media admission.
            |-- audio/                Capture, framing, timeline, bounded sinks, recording, evidence/results.
            |-- live/                 Live session, hotkeys, overlay, injection, runtime, history/recovery.
            |-- runtime/              Route/orchestrator state skeleton.
            `-- stt/
                |-- dispatch.rs       Local fallback runtime state only.
                |-- error.rs          Stable STT error codes/messages.
                |-- model.rs          Shared model cache/download/verification helpers.
                |-- nemotron.rs       Pinned sherpa-onnx Nemotron model bundle.
                |-- parity.rs         Small WER/timestamp helpers for tests.
                `-- settings.rs       Local fallback and compute-target settings.
```

## Current Local Fallback Flow

Official large recordings use or wait for the future `serverBatch` path. The
local path in this branch is the explicit Nemotron INT8 fallback for live/offline
work, plus setup/install/remove controls for the pinned sherpa model artifacts.

1. The React UI calls live/setup commands through `desktop/src/live.ts` and `desktop/src/settings.ts`.
2. Tauri owns mic capture, hotkey state, the live overlay window, and the local fallback runtime.
3. `stt::nemotron` resolves/downloads the pinned Nemotron INT8 artifact set; the live worker keeps one in-process sherpa recognizer warm.
4. Larger recording jobs queue or block until the server contract exists instead of silently producing official-looking local batch transcripts.

Imported recording jobs currently use a transitional React/localStorage queue with Rust-authorized playback paths. It is not durable server execution authority; the Phase 3 connector plan replaces it with Rust-owned string IDs and a SQLite job ledger.

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
- No production `yap-server` connector yet.
