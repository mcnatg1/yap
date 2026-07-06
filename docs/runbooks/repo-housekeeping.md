# Repo housekeeping

This repo stays a staged monorepo through the MVP. Keep cleanup changes small, traceable, and tied to the current client/server plan.

## Layout rules

| Path | Owns | Rule |
|------|------|------|
| `desktop/` | Installed Yap client | React/Tauri app, local Moonshine fallback, desktop tests |
| `desktop/src-tauri/src/live/` | Live dictation runtime | Mic capture, overlay state, hotkey/live stream code |
| `desktop/src-tauri/src/stt/` | Local STT fallback | CrispASR sidecar, model pins, GPU preference, parity helpers |
| `server/` | Future `yap-server` staging | Small API/router code only when it has tests |
| `infra/yap-server-node/` | Server host bootstrap | Host scripts/env examples; no app code |
| `docs/adr/` | Decisions | Why the architecture is this way |
| `docs/specs/` | Build specs | What to implement next |
| `docs/runbooks/` | Operations and maintenance | How to run, audit, clean, or recover things |

## Naming rules

| Surface | Convention | Example |
|---------|------------|---------|
| Desktop npm package | Product package name | `yap-desktop` |
| Tauri Rust package/binary | Product binary name | `yap-desktop.exe` |
| Rust module files | Snake case | `workload_router.rs` |
| React component files | Kebab case | `history-panel.tsx` |
| React component names | PascalCase | `HistoryPanel` |
| Docs | Kebab case except canonical root docs | `server-tier-mvp.md` |
| Server Python package | Snake case | `yap_server` |

Keep the folder name `desktop/` while this is a staged monorepo. Rename the repo/folder only when the Phase 12 split starts.

## Tech debt table

| Priority | Item | Current state | Next action |
|----------|------|---------------|-------------|
| P1 | `desktop/src-tauri/src/lib.rs` is still broad | Filesystem/open/reveal/delete commands moved to `desktop/src-tauri/src/file_actions.rs`; lib.rs still owns setup, live state, tray, and app lifecycle | Split setup/live/tray only when those command clusters change next |
| P1 | Server connector does not exist yet | Batch jobs block/queue instead of routing to `yap-server` | Finish server contract, then add the desktop reachability/connector path |
| P1 | CI parity clip is opt-in | Mock verbose JSON fixture protects timestamp contract in normal CI; real audio sidecar tests are ignored unless `YAP_PARITY_CLIP` is set | Add a licensed speech fixture later if real audio parity must run in CI |
| P2 | ShadCN icon metadata now matches Phosphor | `components.json` declares Phosphor, and app imports Phosphor directly | Keep direct imports; do not add an icon adapter |
| P2 | Active spec filenames use client/server names | Historical phase links were renamed to `local-live-fallback-sidecar.md`, `live-dictation-client-ux.md`, `server-tier-mvp.md`, and `local-llm-sidecar.md` | Leave ADR phase aliases intact unless an ADR is amended |
| P2 | `server/` has a minimal tested slice | Health contract and live/batch workload router exist with Python unittest coverage | Add framework/runtime code only when the server API contract needs it |
| P3 | Local checkout path is historical | `C:\dev\cohere-transcribe-local` differs from repo/product name | Local-only; rename outside Git when convenient |

## Audit commands

```powershell
git diff --check
rg -n "TODO|FIXME|HACK|not implemented|unwired|placeholder|TBD" README.md docs desktop/src desktop/src-tauri/src server
rg -n "CommandCenter|A Tauri App|authors = \[\"you\"\]" README.md docs desktop server infra .github -g "!docs/runbooks/repo-housekeeping.md"
rg -n -F "target\\debug\\desktop.exe" README.md docs desktop server infra .github -g "!docs/runbooks/repo-housekeeping.md"
```
