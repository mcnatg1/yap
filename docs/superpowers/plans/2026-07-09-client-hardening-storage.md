# Client Hardening And Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the remaining Yap desktop hardening without adding SQLite or server connector scope.

**Architecture:** Keep user artifacts as files, keep disposable frontend projections bounded, keep artifact-correlated hidden tombstones reconciled against Rust-owned filesystem checks, and tighten trust boundaries before adding bigger architecture. `App.tsx` remains composition plus still-coupled setup/queue orchestration; `lib.rs` remains bootstrap and command registration while native domains move out only when they have a cohesive owner.

**Tech Stack:** Tauri 2, Rust stdlib, existing Rust crates, React 19, TypeScript, Vitest, Playwright, WebdriverIO.

## Global Constraints

- Do not add SQLite in this phase.
- Do not implement server connector or queued server drain in this phase.
- Do not add a generic storage service.
- Do not migrate transcript/audio bytes into a database.
- Keep local-only code separate from future server streaming code.
- Preserve current behavior unless fixing a bug or closing a native trust-boundary gap.
- A frontend/localStorage path is metadata, not authorization. It must be Rust-minted, restored, or Yap-owned before crossing native open, reveal, transcribe, asset, or future upload boundaries.
- Preserve ADR 0013 focused-field injection and the configurable paste-last shortcut; hardening must not silently remove shipped product behavior.

---

## Current Status (2026-07-09)

| Task | Status | Evidence / remaining gate |
|------|--------|---------------------------|
| 1. Workspace navigation | Complete | Reducer/state tests landed in `f9d2375`; the current worktree adds a pure callback selector proving rail-driven details/polish side effects remain distinct from setup-only `showDetails`. |
| 2. Setup/model projection | Complete | Landed in `8904279` using `desktop/src/lib/setup-model-state.ts`. |
| 3. File trust boundary | Complete for this phase | Core allowlist landed in `9a59f99`; the current worktree adds version rejection, serialized mutation, fail-closed capacity handling, and concurrency coverage. Deliberate registry reclamation UX remains deferred. |
| 4. Focused-field injection | Implemented | The current worktree restores target revalidation, Unicode input, valid-owner clipboard fallback, final-only completed-transcript state, durable paste-last settings, and visible failure feedback. Automated tests pass; a manual cross-application typing smoke remains. |
| 5. Hotkey mutation safety | Complete | Idle guard landed in `792f7e0`; the current worktree adds transactional mutation, rollback, durable startup-failure recovery, conflict handling, and focused tests. |
| 6. Bounded history search | Complete for this phase | Bounds landed in `01270af`; render reset and minimum body-query coverage remain, and the current worktree adds a tested generation guard against late preview writes. |
| 7. Native desktop probes | Complete for this phase | WDIO verifies overlay close resistance, command denial, overlay survival, and the main-window bridge. `test:desktop:all` passes. |
| 8. Storage/SQLite drift | Complete | The reviewed data-class contract retains files and bounded projections now, with explicit SQLite product and measurement gates. |

---

## Commit Strategy

Keep every commit independently buildable:

1. Commit the interrupted-retraction correction with its Playwright/WDIO harness changes as `Harden live overlay behavior and probes`; the regression and the behavior it proves move together and do not depend on the native transcript-delivery patch.
2. Commit the integrated Tasks 3-5 and 8 implementation as `Restore and harden transcript delivery`; `lib.rs`, `live/mod.rs`, `hotkey_commands.rs`, injection, storage reconciliation, frontend wiring, tests, and architecture docs move together.
3. Tasks 1, 2, and 6 retain their own independently buildable commits because they do not share native registration or module references with the integrated slice.

---

## File Structure

Expected files touched by this plan:

- `desktop/src/App.tsx` keeps queue execution and top-level composition; only setup/model extraction should reduce this further in this plan.
- `desktop/src/hooks/use-workspace-navigation.ts` owns workspace navigation state and should receive regression tests.
- `desktop/src/lib/setup-model-state.ts` owns pure fallback/setup projections without hiding queue side effects.
- `desktop/src/history.ts`, `desktop/src/recording-queue.ts`, and `desktop/src/lib/history-render-window.ts` remain the frontend storage seams.
- `desktop/src/components/panels/history-panel.tsx` owns history rendering/search and should enforce the search budget.
- `desktop/src-tauri/src/file_actions.rs` is the current home for file trust-boundary hardening.
- `desktop/src-tauri/src/live/hotkey_commands.rs` owns transactional dictation/paste-last shortcut commands.
- `desktop/src-tauri/src/live/actions.rs` owns completion orchestration; OS-specific insertion stays isolated in `desktop/src-tauri/src/live/injection.rs`.
- `desktop/src-tauri/src/lib.rs` should shrink only by moving cohesive native command domains.
- Tests belong under `desktop/tests/unit`, `desktop/tests/e2e`, and Rust `#[cfg(test)]` modules beside the code under test.

## Task 1: Lock Workspace Navigation Behavior

**Files:**
- Modify: `desktop/src/hooks/use-workspace-navigation.ts`
- Test: `desktop/tests/unit/workspace-navigation.test.ts`
- Verify: `desktop/src/App.tsx`

**Interfaces:**
- Existing: `useWorkspaceNavigation({ onOpenDetails, onOpenPolish })`
- Produces stable behavior for `openWorkspace`, `showDetails`, `closeDetails`, `closeHelp`, `onDetailsOpenChange`, and `onHelpOpenChange`.

- [x] **Step 1: Keep navigation state in the landed pure reducer**

`workspaceNavigationStateForAction` already owns rail, workspace, details/help, and collapse transitions without adding a hook-testing dependency.

- [x] **Step 2: Keep the landed state-transition regression tests**

The existing unit suite covers details/help overlays, close restoration, workspace changes, polish navigation, and setup-only `showDetails` state.

- [x] **Step 3: Extract and test callback selection**

Add `workspaceNavigationEffectForIntent(intent)` beside the reducer. It returns `"refreshDetails"` only for `{ type: "openWorkspace", action: "details" }`, `"openPolish"` only for rail-driven polish, and `undefined` for normal workspaces and `{ type: "showDetails" }`. Make `openWorkspace` use that helper while `showDetails` remains setup-only. Add exact cases to `workspace-navigation.test.ts`.

Run:

```bash
pnpm --dir desktop test -- workspace-navigation.test.ts
```

Expected: tests pass.

- [x] **Step 4: Run build**

Run:

```bash
pnpm --dir desktop build
```

Expected: build passes.

- [ ] **Step 5: Commit**

```bash
git add desktop/src/hooks/use-workspace-navigation.ts desktop/tests/unit/workspace-navigation.test.ts
git commit -m "Test workspace navigation ownership"
```

## Task 2: Keep Setup/Model Projection In The Focused Pure Module

**Files:**
- Modify: `desktop/src/App.tsx`
- Modify: `desktop/src/lib/setup-model-state.ts`
- Test: `desktop/tests/unit/setup-model-state.test.ts`

**Interfaces:**
- Existing App functions: `loadStatus`, `fallbackStatusText`, `maybeOpenSetupPrompt`, `applyFallbackModelView`, `applySetupStatus`, `unblockFallbackReadyQueue`.
- Pure helpers:

```ts
export function fallbackStatusText(view: FallbackModelView, enabled: boolean): string;
export function shouldOpenSetupPrompt(args: {
  fallbackEnabled: boolean;
  setupState: ReturnType<typeof deriveSetupStateFromFallbackModel>;
  alreadyPrompted: boolean;
  skipped: boolean;
}): boolean;
```

- [x] **Step 1: Write pure projection tests first**

Cover:

```ts
it("does not prompt when fallback is disabled");
it("does not prompt when fallback is ready");
it("prompts once when fallback is enabled and missing");
it("unblocks queued fallback jobs when fallback becomes ready");
it("keeps server queued jobs blocked when server connector is unwired");
```

Run:

```bash
pnpm --dir desktop test -- setup-model-state.test.ts
```

The tests were written against the pure contract before the helpers were exported.

- [x] **Step 2: Extract only pure projections**

Pure status text and setup-prompt decisions live in `setup-model-state.ts`. `loadStatus` remains in `App.tsx` because server refresh, live refresh, compute-target loading, and queue unblocking are explicit side effects there.

- [x] **Step 3: Run checks**

Run:

```bash
pnpm --dir desktop test -- setup-model-state.test.ts
pnpm --dir desktop build
```

Expected: tests and build pass.

- [x] **Step 4: Commit**

```bash
git add desktop/src/App.tsx desktop/src/lib/setup-model-state.ts desktop/tests/unit/setup-model-state.test.ts
git commit -m "Extract setup model projections"
```

## Task 3: Harden Rust File Import And Open/Reveal Trust

**Files:**
- Modify: `desktop/src-tauri/src/file_actions.rs`
- Modify: `desktop/src-tauri/src/live/recordings.rs`
- Modify: `desktop/src/history.ts`
- Modify: `desktop/src/hooks/use-transcript-history.ts`
- Modify: `desktop/src/live.ts`
- Test: Rust tests in `desktop/src-tauri/src/file_actions.rs`
- Test: `desktop/tests/unit/history.test.ts`
- Test: `desktop/tests/unit/live.test.ts`

**Interfaces:**
- Existing commands: `allow_recording_playback_path`, `restore_recording_playback_path`, `open_app_path`, `reveal_app_path`, `delete_history_entry_files`, `start_transcribe`.
- Desired boundary: external media paths must be Rust-registered before open/reveal/transcribe; Yap-owned live files remain allowed through live-recordings checks.

- [x] **Step 1: Add failing Rust tests**

Add tests proving:

```rust
#[test]
fn open_app_path_rejects_unregistered_external_media();

#[test]
fn reveal_app_path_rejects_unregistered_external_media();

#[test]
fn registered_external_media_can_be_opened_or_revealed();

#[test]
fn yap_owned_live_transcript_can_be_opened_without_external_registry();
```

Run:

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml file_actions --lib
```

Expected before implementation: unregistered external path tests fail if current behavior still accepts extension-only paths.

- [x] **Step 2: Implement registry validation**

Make `open_app_path` and `reveal_app_path` accept only:

- Yap-owned live transcripts/WAVs under the live recordings directory.
- Paths present in the Rust registry after canonicalization.

Do not trust `localStorage` or frontend-restored paths without Rust validation.

- [x] **Step 3: Validate the transcribe path boundary**

`start_transcribe` keeps its path-based command signature for now, but every external path is canonicalized and checked against the same Rust playback/import registry before dispatch. Yap-owned live paths continue through the owned-directory check.

- [x] **Step 4: Add Rust-authorized tombstone reconciliation**

Only Rust may confirm that a hidden-history tombstone points to a missing, primary Yap-owned live transcript. Process candidates in batches of `200`, preserve all tombstones if a batch fails, and preserve entries added while authorization is pending. After authorization, re-read storage, persist removal of matching stale history metadata first, and remove its tombstone only after that write succeeds.

Rust emits stable canonical live-session paths and resolves existing legacy tombstone aliases. `history.ts` owns Windows comparison identity across case, slash direction, root-clamped dot segments, and verbatim prefixes; stale alias metadata is removed before a tombstone rewrite. Rust and frontend tests cover canonical resolution, equivalent spellings, migration, and hidden-row filtering.

- [x] **Step 5: Run Rust and frontend checks**

Run:

```bash
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
pnpm --dir desktop test
```

Expected: all Rust tests pass.

- [x] **Step 6: Assign files to the integrated Tasks 3-5 and 8 commit**

Do not commit this task in isolation: its native command registration in `lib.rs` and shared frontend bridge in `live.ts` are committed with the injection/hotkey slice.

## Task 4: Preserve And Harden Focused-Field Injection

**Files:**
- Modify: `desktop/src-tauri/src/live/actions.rs`
- Create: `desktop/src-tauri/src/live/injection.rs`
- Modify: `desktop/src-tauri/src/live/recordings.rs`
- Modify: `desktop/src-tauri/src/live/runtime.rs`
- Modify: `desktop/src-tauri/src/live/state.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/src/live/settings.rs`
- Modify: `desktop/src/components/live/live-overlay-state.ts`
- Modify: `desktop/src/components/panels/app-sheets.tsx`
- Modify: `desktop/src/hooks/use-live-control.ts`
- Test: `desktop/tests/unit/live-overlay-state.test.ts`

**Interfaces:**
- Existing behavior: automatic insertion after a successful live stop plus a configurable shortcut that inserts the last transcript.
- Required behavior: capture and revalidate the stop-time external foreground/focused control, insert cleaned final text with Unicode `SendInput` without focusing the overlay, and write the full transcript to the clipboard only when target validation, modifier release, or Windows insertion fails.

- [x] **Step 1: Write regression tests before restoring behavior**

Add Rust tests for:

```rust
#[test]
fn completed_transcript_is_sent_to_the_injection_port();

#[test]
fn startup_shortcut_plan_keeps_dictation_and_paste_hotkeys();

#[test]
fn live_state_restores_paste_hotkey_settings();
```

Keep the OS side effect behind a small injection function so completion orchestration can be tested without typing into the developer's active window.

- [x] **Step 2: Restore settings and persistence**

Keep the paste-last shortcut editable in Settings, reject conflicts with the dictation shortcut, reject shortcut mutation during an active session, persist it atomically, and register it on startup.

- [x] **Step 3: Restore automatic focused-field insertion**

Extract cleaned transcript text before clearing the live session. Preserve it in dedicated Rust-owned completed-transcript state, insert before potentially slower WAV/history writes, then persist the recording normally even when insertion fails. The non-focusable overlay and Yap process must never become the destination.

Start, stop, and stream crash are serialized by the runtime transition gate, and normal stop and stream crash use the same atomic `Saving` finalizer. Tests prove only one finalizer can own PCM, stale start failures cannot block a replacement session, drain-time transcript events preserve completion feedback, injection failure cannot skip saving, and successful retry removes only injection-owned feedback. A crash may inject a completed final transcript; crash, clipboard-fallback, and save messages compose.

- [x] **Step 4: Run checks**

Run:

```bash
pnpm --dir desktop build
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
```

Expected: build and Rust tests pass.

- [ ] **Step 5: Run the Windows cross-application smoke gate**

Use a normal text editor and an elevated text editor. Verify: completed dictation inserts into the field that stayed focused; changing focus during finalization copies without typing into the new field; held shortcut modifiers do not leak into inserted text; an elevated/UIPI target receives clipboard fallback and visible manual-paste feedback; hidden-overlay feedback is temporarily shown; paste-last inserts the last completed final transcript and never an active partial.

- [x] **Step 6: Assign files to the integrated Tasks 3-5 and 8 commit**

The complete native module graph and its frontend wiring move together after the manual smoke gate is recorded as the only remaining release check.

## Task 5: Enforce Hotkey Mutation Safety In Rust

**Files:**
- Modify: `desktop/src-tauri/src/lib.rs`
- Create: `desktop/src-tauri/src/live/hotkey_commands.rs`
- Modify: `desktop/src-tauri/src/live/mod.rs`
- Test: Rust tests beside the live command helpers

**Interfaces:**
- Commands: `set_live_hotkey`, `clear_live_hotkey`, `set_live_paste_hotkey`, `clear_live_paste_hotkey`.

- [x] **Step 1: Add failing tests**

Cover:

```rust
#[test]
fn changing_live_hotkey_while_live_is_started_returns_busy_error();

#[test]
fn clearing_live_hotkey_while_live_is_started_returns_busy_error();

#[test]
fn failed_hotkey_reregister_surfaces_an_error();

#[test]
fn persistence_failure_restores_previous_registration();

#[test]
fn startup_shortcut_plan_deduplicates_conflicting_hotkeys();
```

- [x] **Step 2: Add native idle guard**

Use the same live-session check already used by input device and capture mode commands. UI disabling is not enough.

- [x] **Step 3: Move the cohesive command domain**

Keep transactional dictation and paste-last shortcut registration in `live/hotkey_commands.rs`; startup registration planning remains in `lib.rs`.

- [x] **Step 4: Run checks**

Run:

```bash
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
```

Expected: all Rust tests pass.

- [x] **Step 5: Assign files to the integrated Tasks 3-5 and 8 commit**

`hotkey_commands.rs`, its `live/mod.rs` declaration, and `lib.rs` registration must land in the same buildable commit.

## Task 6: Bound History Search Work

**Files:**
- Modify: `desktop/src/components/panels/history-panel.tsx`
- Modify: `desktop/src/lib/history-preview-loader.ts`
- Test: `desktop/tests/unit/history-preview-loader.test.ts`
- Test: `desktop/tests/unit/history-render-window.test.ts`

**Interfaces:**
- Existing: `renderHistoryWindow(entries, limit)`, `createPreviewTextLoader`, `HistoryPanel`.
- Desired behavior: search across bounded history remains responsive and stale-safe.

- [x] **Step 1: Keep completed search-budget/reset tests**

Cover:

```ts
it("does not load preview text for more than the bounded history cap");
it("does not search transcript bodies for one-character queries");
```

- [x] **Step 2: Add an explicit stale-generation seam**

Add a small generation guard to `history-preview-loader.ts`: `begin()` returns a monotonically increasing token and `isCurrent(token)` becomes false after the next `begin()`. Have the `HistoryPanel` search effect begin a generation and reject late `onLoaded` writes from prior queries. Unit-test the guard directly and keep the existing cancellation cleanup as the component lifecycle backstop.

- [x] **Step 3: Run checks**

Run:

```bash
pnpm --dir desktop test -- history-preview-loader.test.ts history-render-window.test.ts
pnpm --dir desktop build
```

Expected: tests and build pass.

- [ ] **Step 4: Commit**

```bash
git add desktop/src/components/panels/history-panel.tsx desktop/src/lib/history-preview-loader.ts desktop/tests/unit/history-preview-loader.test.ts desktop/tests/unit/history-render-window.test.ts
git commit -m "Bound history search work"
```

## Task 7: Add Native/Desktop Probe Coverage

**Files:**
- Modify: `desktop/tests/playwright.config.ts`
- Modify: `desktop/tests/e2e/live-overlay.spec.ts`
- Modify: `desktop/tests/wdio/live-overlay.spec.js`
- Modify: `desktop/tests/wdio/capabilities/wdio.json`
- Modify if needed: `desktop/package.json`

**Interfaces:**
- Desired coverage: overlay cannot perform main-window-only commands; overlay taskbar/Alt-Tab behavior is probed where supported. File registry enforcement remains covered by Task 3 Rust tests.

- [x] **Step 1: Add WDIO command-boundary tests**

Cover command denial from the live overlay context:

```ts
it("live overlay cannot call main-window-only file actions");
```

- [x] **Step 2: Add WDIO/native lifecycle probes**

Cover:

```ts
it("overlay close attempt does not terminate Yap");
it("overlay stays skip-taskbar or non-user-closeable according to Tauri capabilities");
```

WDIO cannot inspect Alt-Tab/taskbar flags on Windows through the current Tauri bridge. The test file documents that limitation; before release, manually press Alt+Tab while the overlay is visible and verify only the Yap main window appears.

- [x] **Step 3: Run checks**

Run:

```bash
pnpm --dir desktop test
pnpm --dir desktop build
pnpm --dir desktop test:e2e
pnpm --dir desktop test:desktop:all
```

Expected: unit/build pass; desktop tests pass or report an environment-specific limitation with details.

- [ ] **Step 4: Commit**

```bash
git add desktop/tests/playwright.config.ts desktop/tests/e2e/live-overlay.spec.ts desktop/tests/wdio/capabilities/wdio.json desktop/tests/wdio/live-overlay.spec.js
git commit -m "Add desktop hardening probes"
```

## Task 8: Re-check SQLite Decision And Document Drift

**Files:**
- Verify: `docs/superpowers/specs/2026-07-09-client-hardening-storage-design.md`
- Verify: `docs/VOICE-OS-ARCHITECTURE.md`

**Interfaces:**
- Decision: no SQLite until a trigger in the spec is met.

- [x] **Step 1: Run drift scans**

Run:

```bash
rg -n "rusqlite|sqlx|tauri-plugin-sql|SQLite|sqlite" desktop docs
rg -n "localStorage" desktop/src
rg -n "maxTranscriptHistoryEntries|maxStoredQueueJobs|MAX_REGISTERED_PLAYBACK_PATHS|historyRenderWindowSize" desktop/src desktop/src-tauri/src desktop/tests
```

Expected:

- No SQLite dependency added.
- Direct `localStorage` remains limited to focused storage modules plus setup skip until extracted.
- Storage caps remain visible and tested.

- [x] **Step 2: Run full check set**

Run:

```bash
pnpm --dir desktop test
pnpm --dir desktop build
pnpm --dir desktop test:e2e
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
pnpm --dir desktop test:desktop:all
git diff --check
```

Expected: all pass.

- [ ] **Step 3: Commit the integrated Tasks 3-5 and 8 slice**

```bash
git add desktop/src desktop/src-tauri desktop/tests/unit docs/VOICE-OS-ARCHITECTURE.md docs/adr docs/specs/live-dictation-client-ux.md docs/superpowers/specs/2026-07-09-client-hardening-storage-design.md docs/superpowers/plans/2026-07-09-client-hardening-storage.md
git commit -m "Restore and harden transcript delivery"
```

## Self-Review

- Spec coverage: covers SQLite decision, storage seams, App/native split, history search, import trust, and deferred server connector.
- Placeholder scan: no placeholder markers should remain.
- Type consistency: tasks reference existing module names or explicitly name new modules.
- Scope check: server connector, durable job drain, real SQLite migration, and official batch STT remain out of scope for this plan.
