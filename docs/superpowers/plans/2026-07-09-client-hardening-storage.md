# Client Hardening And Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the remaining client maintainability hardening without adding SQLite or server connector scope.

**Architecture:** Keep `App.tsx` as composition, move only cohesive lifecycle groups into hooks/helpers, and keep Rust native domains in small modules. Storage remains bounded JSON/localStorage/files until measured pressure proves SQLite is worth the dependency.

**Tech Stack:** Tauri 2, Rust stdlib, React 19, TypeScript, Vitest, Cargo tests.

## Global Constraints

- Do not add SQLite in this phase.
- Do not implement server connector or queued server drain in this phase.
- Do not add a state-machine, virtualization, or database dependency.
- Keep local-only code separate from future server streaming code.
- Preserve current behavior unless fixing a bug.
- Do not extract `loadStatus` until its app-level side effects are split.

---

### Task 1: Extract Live Control From App

**Files:**
- Create: `desktop/src/hooks/use-live-control.ts`
- Modify: `desktop/src/App.tsx`

**Interfaces:**
- Produces: `useLiveControl(args): { liveView, liveBusy, liveSettingsError, liveInputDevices, updateLiveOverlay, updateLiveHotkey, updateLivePasteHotkey, resetLiveHotkey, clearLiveShortcut, clearLivePasteShortcut, updateLiveCaptureMode, updateInputDevice, preflightLiveInput, startLive, stopLive }`
- Consumes existing functions from `desktop/src/live.ts`.

- [ ] **Step 1: Move only the existing live command wrappers**

Move the current `updateLive`, overlay, hotkey, paste-hotkey, input-device, and start/stop wrappers from `App.tsx` into `use-live-control.ts`. Keep the same command calls.

- [ ] **Step 2: Run frontend checks**

Run: `pnpm --dir desktop build`

Expected: build passes.

- [ ] **Step 3: Commit**

```bash
git add desktop/src/App.tsx desktop/src/hooks/use-live-control.ts
git commit -m "Move live controls out of App"
```

### Task 2: Extract Live Hotkey And Action Glue From lib.rs

**Files:**
- Create: `desktop/src-tauri/src/live/actions.rs`
- Modify: `desktop/src-tauri/src/live/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/src/tray.rs`

**Interfaces:**
- Produces: `live::actions::{show_main_window, start_live_from_app, stop_live_from_app, paste_last_live_transcript, configured_hotkey_matches_shortcut, handle_live_shortcut_action, start_live_runtime, stop_live_runtime}`
- Consumes existing `LiveSessionState`, `LiveRuntime`, `SttState`, and `RuntimeOrchestratorState`.

- [ ] **Step 1: Move native app action and hotkey helpers**

Move the existing helper functions unchanged where possible:

- `show_main_window`
- `start_live_from_app`
- `stop_live_from_app`
- `paste_last_live_transcript`
- `paste_live_text`
- `configured_hotkey_matches_shortcut`
- `handle_live_shortcut_action`
- `start_live_runtime`
- `stop_live_runtime`

- [ ] **Step 2: Update callers**

Update `tray.rs` and hotkey handling in `lib.rs` to call `live::actions::*`.

- [ ] **Step 3: Run Rust checks**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib`

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add desktop/src-tauri/src/lib.rs desktop/src-tauri/src/live/mod.rs desktop/src-tauri/src/live/actions.rs desktop/src-tauri/src/tray.rs
git commit -m "Move live app actions out of lib"
```

### Task 3: Keep History Windowing, Add No Virtualizer

**Files:**
- Modify only if needed: `desktop/src/lib/history-render-window.ts`
- Modify only if needed: `desktop/tests/unit/history-render-window.test.ts`

**Interfaces:**
- Existing: `renderHistoryWindow(entries, limit)`

- [ ] **Step 1: Verify bounded behavior**

Run: `pnpm --dir desktop test -- history-render-window.test.ts`

Expected: tests pass.

- [ ] **Step 2: Do not add measured virtualization**

No code change unless the test fails. The cap is 500, and current rendering is 80 rows per window.

- [ ] **Step 3: Commit only if code changed**

```bash
git status --short
```

Expected: no history virtualization changes unless a regression was found.

### Task 4: Storage Decision Check

**Files:**
- Verify: `docs/superpowers/specs/2026-07-09-client-hardening-storage-design.md`

**Interfaces:**
- Decision: no SQLite until a trigger in the spec is met.

- [ ] **Step 1: Re-check storage caps and direct storage access**

Run:

```bash
rg -n "maxTranscriptHistoryEntries|maxStoredQueueJobs|MAX_REGISTERED_PLAYBACK_PATHS" desktop/src desktop/src-tauri/src
rg -n "localStorage" desktop/src
```

Expected: history, queue, and playback registry remain bounded; direct `localStorage` access stays in `history.ts`, `recording-queue.ts`, or the existing setup skip flag in `App.tsx`.

- [ ] **Step 2: Commit only if drift is found and fixed**

```bash
git add docs/superpowers/specs/2026-07-09-client-hardening-storage-design.md
git commit -m "Document client storage decision"
```

## Self-Review

- Spec coverage: covers `App.tsx`, `lib.rs`, history windowing, and SQLite.
- Placeholder scan: no placeholder markers found.
- Type consistency: planned hook/action names are stable within this plan.
