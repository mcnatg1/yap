# Yap

Yap is a Tauri desktop app for local transcription fallback work. This branch is scoped to the
local Moonshine tiny sidecar foundation: one managed CrispASR process, pinned model artifacts,
loopback bearer auth, progress events, and transcript files written beside source audio.

The old local Python/Cohere runner is gone. Larger recording transcription belongs in the
DGX/server Cohere connector work, not in this local fallback branch.

## Current Posture

- Local/live fallback: Moonshine tiny through a CrispASR sidecar.
- Batch/large recordings: future DGX/server Cohere path.
- Offline without a suitable server/GPU path: queue or block instead of producing low-confidence
  official-looking transcripts.
- Punctuation stays enabled through the pinned FireRed punctuation companion.
- Editor-specific config is not tracked. Use whichever editor you want.

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
|   `-- superpowers/plans/            Historical implementation plans.
`-- desktop/
    |-- README.md                     Desktop-only quick commands.
    |-- package.json                  npm scripts and frontend deps.
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
                |-- settings.rs       Minimal STT settings.
                `-- sidecar.rs        Sidecar process lifecycle, auth, launch args.
```

## Runtime Flow

1. The React UI calls `start_transcribe` through `desktop/src/stt.ts`.
2. Tauri spawns a worker thread in `desktop/src-tauri/src/lib.rs`.
3. `stt::dispatch` serializes files, emits progress, and writes sibling `.txt` files.
4. `stt::sidecar` ensures the pinned CrispASR sidecar, model, tokenizer, and punctuation model are
   present and ready.
5. `stt::crispasr` sends authenticated loopback HTTP requests and returns transcript text.

## Development

```powershell
cd C:\dev\cohere-transcribe-local\desktop
npm install
npm test
npm run build
cargo test --locked --manifest-path .\src-tauri\Cargo.toml
npm run tauri dev
```

Useful narrow checks:

```powershell
cd C:\dev\cohere-transcribe-local
git diff --check
rg -n "transcribe.py|YAP_STT_BACKEND|CommandCenter" -g "!node_modules" -g "!target"
```

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
- No DGX/server connector yet.
