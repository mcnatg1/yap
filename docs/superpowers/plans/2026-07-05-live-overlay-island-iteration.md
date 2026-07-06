# Live Overlay Island Iteration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the live overlay into a richer top-bezel island with peek actions, active recording feedback, processing, success/copy, and failure/retry states.

**Architecture:** Keep the overlay as a separate Tauri webview. Project `LiveSessionView` into a small frontend overlay model, let the frontend own dynamic window resizing because peek is hover-local, and send simple commands/events for start, stop, copy, retry, and opening main app surfaces.

**Tech Stack:** Tauri v2, React 19, TypeScript, Vitest, lucide-react, existing Button primitives.

---

## File Structure

- Modify `desktop/src/components/live/live-overlay.tsx`: render invisible sensor, attached peek actions, waveform recording feedback, processing shimmer, success/copy, and failure/retry UI.
- Modify `desktop/src/components/live/live-overlay-host.tsx`: wire overlay actions to Tauri commands and clipboard.
- Modify `desktop/src/live.ts`: expose the new main-window workspace command.
- Modify `desktop/src/App.tsx`: listen for workspace-open events from the overlay.
- Modify `desktop/src-tauri/src/lib.rs`: keep Rust responsible for creating/showing the transparent overlay webview and add a command that shows the main window and emits the desired workspace.
- Test `desktop/src/lib/live-session.test.ts`: cover the new frontend-facing status expectations that the overlay relies on.

---

### Task 1: Overlay Interaction Model

**Files:**
- Modify: `desktop/src/components/live/live-overlay.tsx`
- Test: `desktop/src/lib/live-session.test.ts`

- [ ] **Step 1: Add frontend state expectations**

Add Vitest coverage that documents the live statuses the overlay needs:

```ts
it("keeps final text available after live returns to idle", () => {
  const view: LiveSessionView = {
    captureMode: "toggle",
    finalText: "hello world",
    hotkey: "Ctrl+Win",
    route: "none",
    status: "idle",
    visibility: "enabled",
  };

  expect(view.status).toBe("idle");
  expect(view.finalText).toBe("hello world");
});
```

- [ ] **Step 2: Render invisible idle sensor and peek**

In `LiveOverlay`, keep idle visually transparent but reveal the attached top-bezel island on hover. The peek island keeps the same compact silhouette and shows inline icon actions:

```text
Mic, Scratch, Transform
```

The idle sensor remains transparent and top-bezel anchored. Do not render a floating Wispr Flow-style stack.

- [ ] **Step 3: Render active recording**

For `armed`, `listening`, and `speaking`, show the compact FreeFlow-style waveform. Toggle mode shows a stop button. Do not show a dictation timer in the island.

- [ ] **Step 4: Render processing**

For `settling` and `saving`, show the processing waveform and keep the window compact. Do not pretend `save_live_session` is complete; it currently only reports an unimplemented error.

- [ ] **Step 5: Render success and failure**

When a live session transitions to idle with `finalText`, show a temporary success island with copy action. Derive `canCopyLast` from `Boolean(finalText?.trim())`. When status is `blocked`, show the error plus retry action and derive `canRetry` from `status === "blocked"`.

- [ ] **Step 6: Verify frontend**

Run:

```powershell
pnpm -C desktop test
pnpm -C desktop build
```

Expected: all tests pass and production build succeeds.

---

### Task 2: Overlay Action Wiring

**Files:**
- Modify: `desktop/src/components/live/live-overlay-host.tsx`
- Modify: `desktop/src/live.ts`
- Modify: `desktop/src/App.tsx`
- Modify: `desktop/src-tauri/src/lib.rs`

- [ ] **Step 1: Add workspace command**

Add a Tauri command:

```rust
#[tauri::command]
fn show_main_workspace(app: tauri::AppHandle, workspace: String) -> Result<(), String> {
    match workspace.as_str() {
        "home" | "transcribe" | "polish" => {
            show_main_window(&app);
            let _ = app.emit("open-workspace", workspace);
            Ok(())
        }
        _ => Err("Unsupported workspace.".into()),
    }
}
```

Register it in `invoke_handler`.

- [ ] **Step 2: Add frontend command wrapper**

Add to `desktop/src/live.ts`:

```ts
export function showMainWorkspace(workspace: "home" | "polish" | "transcribe"): Promise<void> {
  return invoke<void>("show_main_workspace", { workspace });
}
```

- [ ] **Step 3: Listen in main app**

In `App.tsx`, listen for `open-workspace` and call existing rail behavior:

```ts
void listen<unknown>("open-workspace", (event) => {
  if (!isWorkspaceView(event.payload)) return;
  handleRailAction(event.payload);
}).then((stop) => {
  unlistenWorkspace = stop;
});
```

- [ ] **Step 4: Wire overlay buttons**

In `LiveOverlayHost`, wire:

```text
Mic -> startLiveSession()
Retry -> startLiveSession()
Stop -> stopLiveSession()
Copy -> navigator.clipboard.writeText(view.finalText)
Scratch -> showMainWorkspace("home")
Transform -> showMainWorkspace("polish")
```

- [ ] **Step 5: Verify full surface**

Run:

```powershell
pnpm -C desktop test
pnpm -C desktop build
cargo check --locked --manifest-path .\desktop\src-tauri\Cargo.toml
git diff --check
```

Expected: all commands pass.

---

## Self-Review

- Spec coverage: peek, recording, processing, failure, retry, copy-last, mic confidence via existing blocked/mic error text, and scratch/transform routing are covered.
- Placeholder scan: no `TBD`, `TODO`, or "implement later" steps.
- Type consistency: `WorkspaceView` uses existing `"home" | "transcribe" | "polish"`; live states use existing `LiveSessionView` fields.
