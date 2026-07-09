# Client Hardening And Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the remaining Yap desktop hardening without adding SQLite or server connector scope.

**Architecture:** Keep user artifacts as files, keep frontend metadata bounded, and tighten Rust-owned trust boundaries before adding bigger architecture. `App.tsx` remains composition plus still-coupled setup/queue orchestration; `lib.rs` remains bootstrap and command registration while native domains move out only when they have a cohesive owner.

**Tech Stack:** Tauri 2, Rust stdlib, existing Rust crates, React 19, TypeScript, Vitest, Playwright, WebdriverIO.

## Global Constraints

- Do not add SQLite in this phase.
- Do not implement server connector or queued server drain in this phase.
- Do not add a generic storage service.
- Do not migrate transcript/audio bytes into a database.
- Keep local-only code separate from future server streaming code.
- Preserve current behavior unless fixing a bug or closing a native trust-boundary gap.
- No raw frontend/localStorage path can be opened, revealed, transcribed, or asset-authorized unless Rust minted or restored it, or it is Yap-owned.
- Cross-app paste/injection is out of phase unless a later ADR/spec explicitly enables it.

---

## File Structure

Expected files touched by this plan:

- `desktop/src/App.tsx` keeps queue execution and top-level composition; only setup/model extraction should reduce this further in this plan.
- `desktop/src/hooks/use-workspace-navigation.ts` owns workspace navigation state and should receive regression tests.
- `desktop/src/hooks/use-setup-model-state.ts` may be created only if it cleanly owns fallback model/setup projections without hiding queue side effects.
- `desktop/src/history.ts`, `desktop/src/recording-queue.ts`, and `desktop/src/lib/history-render-window.ts` remain the frontend storage seams.
- `desktop/src/components/panels/history-panel.tsx` owns history rendering/search and should enforce the search budget.
- `desktop/src-tauri/src/file_actions.rs` is the current home for file trust-boundary hardening.
- `desktop/src-tauri/src/live/commands.rs` or `desktop/src-tauri/src/live/hotkey_commands.rs` may be created for live command/hotkey mutation code.
- `desktop/src-tauri/src/live/actions.rs` must not grow into a cross-app injection dumping ground.
- `desktop/src-tauri/src/lib.rs` should shrink only by moving cohesive native command domains.
- Tests belong under `desktop/tests/unit`, `desktop/tests/e2e`, and Rust `#[cfg(test)]` modules beside the code under test.

## Task 1: Lock Workspace Navigation Behavior

**Files:**
- Modify: `desktop/src/hooks/use-workspace-navigation.ts`
- Test: `desktop/tests/unit/workspace-navigation.test.ts` or pure reducer test if hook testing requires new tooling
- Verify: `desktop/src/App.tsx`

**Interfaces:**
- Existing: `useWorkspaceNavigation({ onOpenDetails, onOpenPolish })`
- Produces stable behavior for `openWorkspace`, `showDetails`, `closeDetails`, `closeHelp`, `onDetailsOpenChange`, and `onHelpOpenChange`.

- [ ] **Step 1: Prefer a pure reducer if hook testing would add a dependency**

If Vitest cannot test hooks without adding a library, extract a pure reducer:

```ts
export type WorkspaceNavigationState = {
  activeRail: RailAction;
  detailsOpen: boolean;
  helpOpen: boolean;
  railCollapsed: boolean;
  workspaceView: WorkspaceView;
};

export function workspaceNavigationStateForAction(
  state: WorkspaceNavigationState,
  action: { type: "open"; rail: RailAction } | { type: "closeDetails" } | { type: "closeHelp" },
): WorkspaceNavigationState {
  if (action.type === "closeDetails") {
    return {
      ...state,
      activeRail: state.activeRail === "details" ? state.workspaceView : state.activeRail,
      detailsOpen: false,
    };
  }
  if (action.type === "closeHelp") {
    return {
      ...state,
      activeRail: state.activeRail === "help" ? state.workspaceView : state.activeRail,
      helpOpen: false,
    };
  }
  if (action.rail === "details") return { ...state, activeRail: "details", detailsOpen: true };
  if (action.rail === "help") return { ...state, activeRail: "help", helpOpen: true };
  return { ...state, activeRail: action.rail, workspaceView: action.rail };
}
```

- [ ] **Step 2: Add regression tests**

Cover these cases:

```ts
it("opens details without changing the workspace view");
it("restores the active rail to the current workspace when details closes");
it("opens help without changing the workspace view");
it("opens polish and leaves the polish side effect injected by App");
it("showDetails opens setup without triggering the refresh callback");
```

Run:

```bash
pnpm --dir desktop test -- workspace-navigation.test.ts
```

Expected: tests pass.

- [ ] **Step 3: Run build**

Run:

```bash
pnpm --dir desktop build
```

Expected: build passes.

- [ ] **Step 4: Commit**

```bash
git add desktop/src/hooks/use-workspace-navigation.ts desktop/tests/unit/workspace-navigation.test.ts
git commit -m "Test workspace navigation ownership"
```

## Task 2: Split Setup/Model Projection From App Only If It Reduces Coupling

**Files:**
- Modify: `desktop/src/App.tsx`
- Create if justified: `desktop/src/hooks/use-setup-model-state.ts`
- Test: `desktop/tests/unit/setup-model-state.test.ts`

**Interfaces:**
- Existing App functions: `loadStatus`, `fallbackStatusText`, `maybeOpenSetupPrompt`, `applyFallbackModelView`, `applySetupStatus`, `unblockFallbackReadyQueue`.
- New pure helpers, if created:

```ts
export function fallbackStatusText(view: FallbackModelView, enabled: boolean): string;
export function shouldOpenSetupPrompt(args: {
  fallbackEnabled: boolean;
  setupState: ReturnType<typeof deriveSetupStateFromFallbackModel>;
  alreadyPrompted: boolean;
  skipped: boolean;
}): boolean;
```

- [ ] **Step 1: Write pure projection tests first**

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

Expected before implementation: tests fail because helpers are not exported.

- [ ] **Step 2: Extract only pure projections**

Move pure status text and setup-prompt decision logic out of `App.tsx`. Keep `loadStatus` in `App.tsx` unless the hook can receive explicit callbacks for `refreshServerState`, `refreshLiveState`, `loadComputeTargets`, and queue unblocking without hiding side effects.

- [ ] **Step 3: Run checks**

Run:

```bash
pnpm --dir desktop test -- setup-model-state.test.ts
pnpm --dir desktop build
```

Expected: tests and build pass.

- [ ] **Step 4: Commit**

```bash
git add desktop/src/App.tsx desktop/src/hooks/use-setup-model-state.ts desktop/tests/unit/setup-model-state.test.ts
git commit -m "Extract setup model projections"
```

## Task 3: Harden Rust File Import And Open/Reveal Trust

**Files:**
- Modify: `desktop/src-tauri/src/file_actions.rs`
- Modify if needed: `desktop/src/lib/playback-registry.ts`
- Modify if needed: `desktop/src/App.tsx`
- Test: Rust tests in `desktop/src-tauri/src/file_actions.rs`

**Interfaces:**
- Existing commands: `allow_recording_playback_path`, `restore_recording_playback_path`, `open_app_path`, `reveal_app_path`, `delete_history_entry_files`, `start_transcribe`.
- Desired boundary: external media paths must be Rust-registered before open/reveal/transcribe; Yap-owned live files remain allowed through live-recordings checks.

- [ ] **Step 1: Add failing Rust tests**

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

- [ ] **Step 2: Implement registry validation**

Make `open_app_path` and `reveal_app_path` accept only:

- Yap-owned live transcripts/WAVs under the live recordings directory.
- Paths present in the Rust registry after canonicalization.

Do not trust `localStorage` or frontend-restored paths without Rust validation.

- [ ] **Step 3: Decide transcribe path boundary**

If `start_transcribe` still takes raw paths, either:

- Validate each path against the same Rust import registry before dispatch; or
- Defer changing the command signature but add an explicit spec note and a failing ignored test for import IDs.

Prefer validation now if it is small.

- [ ] **Step 4: Run Rust checks**

Run:

```bash
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
```

Expected: all Rust tests pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/file_actions.rs desktop/src/lib/playback-registry.ts desktop/src/App.tsx
git commit -m "Harden recording file trust boundary"
```

## Task 4: Remove Or Gate Cross-App Paste From The Foundation

**Files:**
- Modify: `desktop/src-tauri/src/live/actions.rs`
- Delete if still present: `desktop/src-tauri/src/live/paste.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/src/live/settings.rs`
- Modify: `desktop/src/components/panels/app-sheets.tsx`
- Modify: `desktop/src/hooks/use-live-control.ts`

**Interfaces:**
- Current behavior: `paste_last_live_transcript`, paste hotkey, and automatic paste after stop.
- Desired behavior for this phase: no cross-app injection unless explicitly gated behind a disabled future flag.

- [ ] **Step 1: Write tests for disabled paste behavior**

Add Rust tests for:

```rust
#[test]
fn stop_live_runtime_does_not_auto_paste_when_injection_is_disabled();

#[test]
fn paste_hotkey_is_not_registered_when_injection_is_disabled();
```

If direct global-shortcut registration is hard to test, extract a pure registration-plan helper and test that.

- [ ] **Step 2: Remove or gate UI**

Remove paste hotkey controls from Settings unless a disabled "future feature" flag already exists. Do not leave dead controls that imply paste is ready.

- [ ] **Step 3: Remove auto-paste on stop**

Make stop/save/copy behavior explicit and non-invasive. If "copy last transcript" remains, it must copy to clipboard only when the user invokes a visible command and it must not send `Ctrl+V`.

- [ ] **Step 4: Run checks**

Run:

```bash
pnpm --dir desktop build
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
```

Expected: build and Rust tests pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/live/actions.rs desktop/src-tauri/src/live/mod.rs desktop/src-tauri/src/live/paste.rs desktop/src-tauri/src/lib.rs desktop/src-tauri/src/live/settings.rs desktop/src/components/panels/app-sheets.tsx desktop/src/hooks/use-live-control.ts desktop/src/live.ts
git commit -m "Gate cross app paste behavior"
```

## Task 5: Enforce Hotkey Mutation Safety In Rust

**Files:**
- Modify: `desktop/src-tauri/src/lib.rs`
- Create if useful: `desktop/src-tauri/src/live/commands.rs`
- Modify: `desktop/src-tauri/src/live/mod.rs`
- Test: Rust tests beside the live command helpers

**Interfaces:**
- Commands: `set_live_hotkey`, `clear_live_hotkey`, `set_live_capture_mode`, `set_input_device`.

- [ ] **Step 1: Add failing tests**

Cover:

```rust
#[test]
fn changing_live_hotkey_while_live_is_started_returns_busy_error();

#[test]
fn clearing_live_hotkey_while_live_is_started_returns_busy_error();

#[test]
fn failed_hotkey_reregister_surfaces_an_error();
```

- [ ] **Step 2: Add native idle guard**

Use the same live-session check already used by input device and capture mode commands. UI disabling is not enough.

- [ ] **Step 3: Move command helpers only if cohesive**

If the touched command group exceeds a small patch, move live hotkey command helpers into `live/commands.rs`; otherwise keep the patch focused and leave the split for a later pass.

- [ ] **Step 4: Run checks**

Run:

```bash
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
```

Expected: all Rust tests pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/lib.rs desktop/src-tauri/src/live/commands.rs desktop/src-tauri/src/live/mod.rs
git commit -m "Guard live hotkey mutations"
```

## Task 6: Bound History Search Work

**Files:**
- Modify: `desktop/src/components/panels/history-panel.tsx`
- Modify if needed: `desktop/src/lib/history-preview-loader.ts`
- Test: `desktop/tests/unit/history-panel-search.test.ts` or a pure helper test

**Interfaces:**
- Existing: `renderHistoryWindow(entries, limit)`, `createPreviewTextLoader`, `HistoryPanel`.
- Desired behavior: search across bounded history remains responsive and stale-safe.

- [ ] **Step 1: Add tests around search/reset behavior**

Cover:

```ts
it("resets the render limit when the search query changes");
it("does not load preview text for more than the bounded history cap");
it("ignores stale preview results after query changes");
it("does not search transcript bodies for one-character queries if that budget is adopted");
```

- [ ] **Step 2: Implement the smallest budget control**

Prefer one of these, in order:

- Keep current full-history search but prove it is capped at `500`.
- Add a minimum transcript-body query length such as `2`.
- Add debounce only if tests or traces show repeated preview loads.

- [ ] **Step 3: Run checks**

Run:

```bash
pnpm --dir desktop test -- history-panel-search.test.ts history-render-window.test.ts
pnpm --dir desktop build
```

Expected: tests and build pass.

- [ ] **Step 4: Commit**

```bash
git add desktop/src/components/panels/history-panel.tsx desktop/src/lib/history-preview-loader.ts desktop/tests/unit/history-panel-search.test.ts
git commit -m "Bound history search work"
```

## Task 7: Add Native/Desktop Probe Coverage

**Files:**
- Modify: `desktop/tests/e2e/*.spec.ts`
- Modify if available: `desktop/tests/wdio/*.ts`
- Modify if needed: `desktop/package.json`

**Interfaces:**
- Desired coverage: overlay cannot perform main-window-only commands; overlay taskbar/Alt-Tab behavior is probed where supported; path registry enforcement is visible to tests.

- [ ] **Step 1: Add Playwright bridge tests where possible**

Cover command denial from the live overlay context:

```ts
test("live overlay cannot call main-window-only file actions");
```

- [ ] **Step 2: Add WDIO/native probes if the harness already runs locally**

Cover:

```ts
it("overlay close attempt does not terminate Yap");
it("overlay stays skip-taskbar or non-user-closeable according to Tauri capabilities");
```

If WDIO cannot inspect Alt-Tab/taskbar flags on Windows, document that limitation in the test file comment and keep a manual check command.

- [ ] **Step 3: Run checks**

Run:

```bash
pnpm --dir desktop test
pnpm --dir desktop build
pnpm --dir desktop test:desktop:all
```

Expected: unit/build pass; desktop tests pass or report an environment-specific limitation with details.

- [ ] **Step 4: Commit**

```bash
git add desktop/tests desktop/package.json
git commit -m "Add desktop hardening probes"
```

## Task 8: Re-check SQLite Decision And Document Drift

**Files:**
- Verify: `docs/superpowers/specs/2026-07-09-client-hardening-storage-design.md`
- Verify: `docs/VOICE-OS-ARCHITECTURE.md`

**Interfaces:**
- Decision: no SQLite until a trigger in the spec is met.

- [ ] **Step 1: Run drift scans**

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

- [ ] **Step 2: Run full check set**

Run:

```bash
pnpm --dir desktop test
pnpm --dir desktop build
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
git diff --check
```

Expected: all pass.

- [ ] **Step 3: Commit docs if they changed**

```bash
git add docs/superpowers/specs/2026-07-09-client-hardening-storage-design.md docs/superpowers/plans/2026-07-09-client-hardening-storage.md docs/VOICE-OS-ARCHITECTURE.md
git commit -m "Refresh client hardening plan"
```

## Self-Review

- Spec coverage: covers SQLite decision, storage seams, App/native split, history search, import trust, and deferred server connector.
- Placeholder scan: no placeholder markers should remain.
- Type consistency: tasks reference existing module names or explicitly name new modules.
- Scope check: server connector, durable job drain, real SQLite migration, and official batch STT remain out of scope for this plan.
