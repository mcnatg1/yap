# Yap

Yap is a staged monorepo for the MVP: one desktop app, one future server tier, and the docs that keep the architecture honest.

The desktop is a Tauri app with a local Moonshine v2 tiny fallback. Larger recording transcription belongs on the GB-class server path once `yap-server` exists. The long-term split is still `yap-desktop`, `yap-server`, and `yap-knowledge` (ADR 0018), but staying in this repo through MVP avoids cross-repo churn while the server contract is still moving.

## Current Posture

- Local/live fallback: Moonshine v2 tiny through a CrispASR sidecar.
- Local fallback install: explicit setup/settings action; runtime never silently downloads models.
- Client workflow: typed recording-job state for setup, local fallback, future server routing, preprocessing, and diarization.
- Batch/large recordings: future GB-class server Cohere path.
- Offline without a suitable server/GPU path: queue or block instead of producing low-confidence
  official-looking transcripts.
- Punctuation stays enabled through the pinned FireRed punctuation companion.
- Editor-specific config is not tracked. Use whichever editor you want.
- MVP repo posture: staged monorepo, no Nx/Turborepo, no separate `yap-contracts` yet.

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
|   `-- superpowers/plans/            Historical implementation plans.
|-- infra/
|   `-- yap-server-node/              GB-class node setup script and env example.
|-- server/
|   `-- README.md                     MVP staging area for future yap-server work.
`-- desktop/
    |-- README.md                     Desktop-only quick commands.
    |-- package.json                  pnpm scripts and frontend deps.
    |-- vite.config.ts                Vite/Tauri dev server config.
    |-- crispasr-version.txt          Pinned CrispASR/model/tokenizer/punctuation artifacts.
    |-- src/                          React app.
    |   |-- App.tsx                   Main app state and screen composition.
    |   |-- stt.ts                    Tauri STT event/invoke client.
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
        |-- nsis-hooks.nsh            Windows installer DLL hook.
        |-- tests/parity.rs           Sidecar smoke/parity checks.
        `-- src/
            |-- lib.rs                Tauri commands, event wiring, app lifecycle.
            `-- stt/
                |-- binary.rs         CrispASR binary resolution/install verification.
                |-- crispasr.rs       HTTP transcription client and retry path.
                |-- dispatch.rs       Batch orchestration and transcript file writes.
                |-- error.rs          Stable STT error codes/messages.
                |-- gpu.rs            GPU detection/preference.
                |-- model.rs          Model cache/download/verification.
                |-- parity.rs         Small WER/timestamp helpers for tests.
                |-- pin.rs            `crispasr-version.txt` parser.
                |-- progress.rs       Progress event structs.
                |-- settings.rs       Env-only GPU preference (`YAP_USE_GPU`); no persisted app settings.
                `-- sidecar.rs        Sidecar process lifecycle, auth, launch args.
```

## Runtime Flow

1. The React UI calls `start_transcribe` through `desktop/src/stt.ts`.
2. Tauri spawns a worker thread in `desktop/src-tauri/src/lib.rs`.
3. `stt::dispatch` serializes files, emits progress, and writes sibling `.txt` files.
4. `stt::sidecar` uses the already-installed CrispASR sidecar, model, tokenizer, and punctuation
   model; setup/settings own downloads and removal.
5. `stt::crispasr` sends authenticated loopback HTTP requests and returns transcript text.

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

`pnpm test:desktop` expects `src-tauri\target\debug\desktop.exe` unless `APP_BINARY` points at
another build. The WDIO hooks are only compiled when `test:desktop:build` uses the `wdio` feature
and `src-tauri\tauri.wdio.conf.json`.

Useful narrow checks:

```powershell
cd C:\dev\cohere-transcribe-local
git diff --check
rg -n "transcribe.py|CommandCenter" -g "!node_modules" -g "!target"
```

## MVP Monorepo Rule

Keep code here until the first server path is real enough to split:

- `desktop/` owns the installed client and local fallback.
- `server/` stages the Phase 8 `yap-server` contract and first service code.
- `infra/yap-server-node/` owns host/bootstrap setup for GB-class server nodes.
- `docs/` stays authoritative while the architecture is still moving.

Do not add monorepo tooling until two packages actually need shared commands or dependency graphing. `pnpm -C desktop ...` and direct server commands are enough for now.

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
- No runtime backend selector.
- No command palette or generated UI drawer.
- No editor-specific project config.
- No production `yap-server` connector yet.
