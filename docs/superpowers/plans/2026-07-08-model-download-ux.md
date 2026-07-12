# Model Download UX Implementation Plan

> **Implementation status (2026-07-12):** Baseline implemented: install, progress, cancel, verify, remove, disable/enable, and open-folder flows exist. Model-artifact licensing, hosted real-model/native CI, and production-release proof remain; task boxes below are historical execution notes.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the single local fallback model lifecycle explicit in setup and settings: install, progress, cancel, verify, remove, disable, enable, and open folder.

**Architecture:** `desktop/src-tauri/src/stt/nemotron.rs` owns the pinned Nemotron model identity and status projection. `desktop/src-tauri/src/stt/model.rs` owns reusable file, hash, and chunked download primitives. `desktop/src-tauri/src/lib.rs` exposes thin Tauri commands and events. React consumes one typed `FallbackModelView` and renders one compact model section in settings.

**Tech Stack:** Rust, Tauri 2, `reqwest::blocking`, `serde`, React, TypeScript, existing shadcn-style primitives, Vitest, Cargo unit tests.

## Global Constraints

- Runtime must never silently download model files.
- The desktop owns exactly one local fallback model: Nemotron 3.5 ASR Streaming 0.6B INT8 through `sherpa-onnx`.
- Do not add Whisper, Parakeet, Moonshine, Cohere local selectors, or client-side GPU routing.
- Do not add a UI package; use existing primitives and local components.
- Destructive removal must use the existing alert dialog pattern.
- Corrupt or partial installs fail closed and must not load.
- Removing the model disables local fallback until the user explicitly reinstalls it.
- Do not add server-side routing, batch ASR, diarization, fusion, or server model management in this plan.
- Keep Tauri command bodies thin; implementation belongs in `stt/`.
- Keep user-facing labels short. Put technical detail in tooltips, docs, or status details.
- Preserve the current Node 24 / pnpm tooling assumptions.

---

## Task 1: Add A Stable Rust Status Contract

**Files**

- `desktop/src-tauri/src/stt/nemotron.rs`
- `desktop/src-tauri/src/stt/error.rs`
- `desktop/src-tauri/src/stt/mod.rs`

**Interfaces**

Add the command-facing view types in `nemotron.rs`:

```rust
pub const MODEL_ID: &str = "nemotron-3.5-asr-streaming-0.6b-1120ms-int8";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FallbackModelStatus {
    Missing,
    Downloading,
    Verifying,
    Ready,
    Corrupted,
    Disabled,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FallbackModelView {
    pub id: String,
    pub label: String,
    pub status: FallbackModelStatus,
    pub installed_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub progress_percent: Option<f32>,
    pub speed_mbps: Option<f32>,
    pub message: Option<String>,
    pub models_dir: String,
}
```

Add status helpers:

```rust
pub fn model_status(enabled: bool) -> FallbackModelView;
pub fn model_status_at(root: &std::path::Path, enabled: bool) -> FallbackModelView;
pub fn verify_model(enabled: bool) -> FallbackModelView;
```

**Steps**

- [ ] Add `MODEL_ID` beside the existing `MODEL_LABEL`.
- [ ] Add `FallbackModelStatus` and `FallbackModelView` with serialized values matching the TypeScript union in `docs/specs/model-download-ux.md`.
- [ ] Add `model_status_at(root, enabled)` so tests can use a temp directory without changing process environment.
- [ ] Add an artifact classifier that distinguishes `missing`, `ready`, and `corrupted`; do not build status from the current boolean `is_installed()`.
- [ ] Make status classification read-only. It may read marker files and file metadata, but it must not hash large artifacts or write `.verified` markers.
- [ ] Keep hash-and-marker repair inside explicit verify, install, repair, and reinstall paths.
- [ ] Treat any missing artifact as `missing`.
- [ ] Treat any artifact with a failed hash or stale verified marker as `corrupted`.
- [ ] Treat all verified artifacts as `ready` when enabled and `disabled` when disabled.
- [ ] Keep `resolve_model()` strict: it returns paths only when artifacts are present and verified, and it returns `ModelCorrupt` for corrupt artifacts rather than collapsing them to `ModelMissing`.
- [ ] Add `SttError::ModelInstallCancelled` only if the download primitive needs a stable cancellation error.
- [ ] Add unit tests for missing, ready, disabled, and corrupted status projection.
- [ ] Add unit tests for downloading, verifying, and error status projection using synthetic progress/error inputs.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml nemotron
```

---

## Task 2: Add Chunked Download Progress And Cancellation

**Files**

- `desktop/src-tauri/src/stt/model.rs`
- `desktop/src-tauri/src/stt/nemotron.rs`
- `desktop/src-tauri/src/stt/error.rs`

**Interfaces**

Add progress primitives in `model.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub elapsed_ms: u128,
}

impl DownloadProgress {
    pub fn percent(self) -> Option<f32>;
    pub fn speed_mbps(self) -> Option<f32>;
}

pub fn download_file_with_progress<P, C>(
    url: &str,
    dest: &std::path::Path,
    on_progress: P,
    is_cancelled: C,
) -> Result<(), SttError>
where
    P: FnMut(DownloadProgress),
    C: Fn() -> bool;
```

Keep the existing `download_file(url, dest)` as a wrapper for tests and legacy callers:

```rust
pub fn download_file(url: &str, dest: &std::path::Path) -> Result<(), SttError> {
    download_file_with_progress(url, dest, |_| {}, || false)
}
```

**Steps**

- [ ] Refactor the current `std::io::copy` path into a manual read/write loop with a 64 KiB buffer.
- [ ] Emit progress after each successful write and once after the final rename.
- [ ] Read `content_length()` from the response when available.
- [ ] Write to `*.part` first, then atomically rename on success.
- [ ] If cancellation is requested, delete the partial file and return `ModelInstallCancelled`.
- [ ] If any non-cancel write, network, or rename failure happens mid-download, delete the `.part` file unless safe byte-range resume has been implemented and tested.
- [ ] After cancellation or failure, leave no verified marker and leave status `missing` or `error`.
- [ ] Add a small pure helper for progress math so unit tests do not need network access.
- [ ] Update `nemotron::ensure_artifact` to call the progress variant from a new `ensure_model_with_progress`.
- [ ] Add a `force` mode for reinstall and repair paths; force mode removes stale artifact files, `.verified` markers, and `.part` files before downloading.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml model
```

---

## Task 3: Expose Thin Tauri Commands And Events

**Files**

- `desktop/src-tauri/src/lib.rs`
- `desktop/src-tauri/src/stt/nemotron.rs`
- `desktop/src-tauri/src/stt/error.rs`

**Interfaces**

Add commands:

```rust
#[tauri::command]
fn fallback_model_status(
    install_state: tauri::State<'_, FallbackModelInstallState>,
) -> Result<FallbackModelView, crate::stt::dispatch::SttCommandError>;

#[tauri::command]
async fn fallback_model_install(
    app: tauri::AppHandle,
    install_state: tauri::State<'_, FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<FallbackModelView, crate::stt::dispatch::SttCommandError>;

#[tauri::command]
fn fallback_model_cancel_install(
    install_state: tauri::State<'_, FallbackModelInstallState>,
) -> Result<FallbackModelView, crate::stt::dispatch::SttCommandError>;

#[tauri::command]
fn fallback_model_verify(
    app: tauri::AppHandle,
    install_state: tauri::State<'_, FallbackModelInstallState>,
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<FallbackModelView, crate::stt::dispatch::SttCommandError>;

#[tauri::command]
fn fallback_model_remove(
    live_state: tauri::State<'_, live::LiveSessionState>,
) -> Result<FallbackModelView, crate::stt::dispatch::SttCommandError>;

#[tauri::command]
fn fallback_model_set_enabled(
    live_state: tauri::State<'_, live::LiveSessionState>,
    enabled: bool,
) -> Result<FallbackModelView, crate::stt::dispatch::SttCommandError>;

#[tauri::command]
fn fallback_model_open_folder(app: tauri::AppHandle) -> Result<(), crate::stt::dispatch::SttCommandError>;
```

Use these event names:

```rust
const FALLBACK_MODEL_STATUS_EVENT: &str = "fallback-model-status";
const FALLBACK_MODEL_PROGRESS_EVENT: &str = "fallback-model-progress";
```

**Steps**

- [ ] Use the existing backend error style: `crate::stt::dispatch::SttCommandError`.
- [ ] Use existing live guards by accepting `live::LiveSessionState` for install, verify, remove, and enable/disable actions.
- [ ] Add `FallbackModelInstallState` with current phase/view/progress/error plus a `Mutex<Option<Arc<AtomicBool>>>` cancellation token.
- [ ] Reject or coalesce concurrent install/verify/reinstall actions; never run two model lifecycle workers at the same time.
- [ ] Clear transient install state on worker exit after publishing the final status.
- [ ] Register the state with the Tauri builder.
- [ ] Add the new commands to `tauri::generate_handler!`.
- [ ] Implement `fallback_model_install` with `tauri::async_runtime::spawn_blocking`.
- [ ] During install, emit `fallback-model-progress` payloads using `FallbackModelView`.
- [ ] Throttle progress events by elapsed time or percent change; always emit first and final progress, and never emit non-finite `progressPercent` or `speedMbps`.
- [ ] After install, verify all artifacts, emit `fallback-model-status`, and return the final view.
- [ ] During explicit verify, emit `verifying` through `fallback-model-status`; if hashing takes noticeable time, emit `fallback-model-progress` after each artifact.
- [ ] Make `fallback_model_cancel_install` idempotent; cancelling with no active install returns current status.
- [ ] When cancellation interrupts an active install, the install worker must publish one final status event of `missing` or `error`, and the frontend cancel action must refresh status after requesting cancellation.
- [ ] Make `fallback_model_remove` disable fallback explicitly after successful removal.
- [ ] Implement `fallback_model_open_folder` with a model-folder-specific allowlist for `nemotron::root_dir()`; the existing media/transcript `open_app_path` and `reveal_app_path` helpers do not allow model folders as-is.
- [ ] Preserve legacy commands `setup_status`, `install_local_fallback`, `remove_local_fallback`, and `set_local_fallback_enabled` until the frontend has migrated.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml fallback_model
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml setup
```

---

## Task 4: Add Typed Frontend Model Lifecycle Calls

**Files**

- `desktop/src/settings.ts`
- `desktop/src/lib/app-types.ts`
- `desktop/src/App.tsx`

**Interfaces**

Add TypeScript types:

```ts
export type FallbackModelStatus =
  | "missing"
  | "downloading"
  | "verifying"
  | "ready"
  | "corrupted"
  | "disabled"
  | "error";

export type FallbackModelView = {
  id: "nemotron-3.5-asr-streaming-0.6b-1120ms-int8";
  label: string;
  status: FallbackModelStatus;
  installedBytes?: number | null;
  totalBytes?: number | null;
  progressPercent?: number | null;
  speedMbps?: number | null;
  message?: string | null;
  modelsDir: string;
};
```

Add invoke wrappers in `settings.ts`:

```ts
export function fallbackModelStatus(): Promise<FallbackModelView>;
export function installFallbackModel(): Promise<FallbackModelView>;
export function cancelFallbackModelInstall(): Promise<FallbackModelView>;
export function verifyFallbackModel(): Promise<FallbackModelView>;
export function removeFallbackModel(): Promise<FallbackModelView>;
export function setFallbackModelEnabled(enabled: boolean): Promise<FallbackModelView>;
export function openFallbackModelFolder(): Promise<void>;
export function listenFallbackModelProgress(
  onProgress: (view: FallbackModelView) => void,
): Promise<() => void>;
export function listenFallbackModelStatus(
  onStatus: (view: FallbackModelView) => void,
): Promise<() => void>;
```

**Steps**

- [ ] Move the model lifecycle TypeScript types into `app-types.ts` or `settings.ts`; choose the location with the least circular import pressure.
- [ ] Keep existing setup types until the UI no longer depends on `SetupStatus`.
- [ ] Load `fallbackModelStatus()` during app boot beside `setup_status()`.
- [ ] Follow the existing `desktop/src/live.ts` wrapper/listener pattern instead of adding raw `listen()` calls in `App.tsx`.
- [ ] Subscribe to both `fallback-model-progress` and `fallback-model-status` in the same effect that subscribes to live events.
- [ ] Map the new status to the existing `SetupState` only where legacy UI still needs it.
- [ ] Update app state from the returned `FallbackModelView` after install, cancel, verify, remove, enable, and disable.
- [ ] After `cancelFallbackModelInstall()`, call `fallbackModelStatus()` unless the command already returned a terminal `missing` or `error` view.
- [ ] Avoid duplicated busy booleans by deriving busy from `fallbackModel.status === "downloading" || fallbackModel.status === "verifying"` plus any in-flight command promise.

**Verification**

```powershell
cd .\desktop
pnpm test -- app-types settings
pnpm typecheck
```

---

## Task 5: Render The Settings Lifecycle UI

**Files**

- `desktop/src/components/panels/app-sheets.tsx`
- `desktop/src/App.tsx`
- `desktop/src/components/ui/alert-dialog.tsx`

**UI States**

| Status | Primary action | Secondary action | Detail |
|--------|----------------|------------------|--------|
| `missing` | Install | Open folder | Local fallback is not installed. |
| `downloading` | Cancel | Open folder | Downloading percent and speed. |
| `verifying` | None | Open folder | Verifying files. |
| `ready` | Reinstall | Verify, Disable, Remove | Ready. |
| `corrupted` | Repair | Remove | Files failed verification. |
| `disabled` | Enable | Remove | Disabled. |
| `error` | Retry | Remove | Actionable error message. |

**Steps**

- [ ] Replace the current generic "Model files" and "Storage" rows with one compact lifecycle group.
- [ ] Keep copy short: "Ready", "Disabled", "Files failed verification.", "Downloading 42%".
- [ ] Use `Progress` only if already present; otherwise use text percent and avoid adding a dependency.
- [ ] Disable install/remove/verify actions while live recording is active.
- [ ] Treat `saving` as active for settings disablement, matching backend live guards.
- [ ] Disable cancel unless status is `downloading`.
- [ ] In `ready`, expose `Reinstall` as the primary action and keep `Verify`, `Disable`, and `Remove` discoverable secondary actions.
- [ ] In `corrupted`, make `Repair` perform delete-partials, download missing or corrupt artifacts, and verify all artifacts before returning `ready`.
- [ ] Use the alert dialog for `Remove`; copy must state that local live fallback will be unavailable until reinstalled.
- [ ] Keep `Open folder` available for all states.
- [ ] Preserve setup prompt behavior, base its state on `FallbackModelView`, and give the setup prompt the same install, cancel, retry, and open-folder controls for the current state. Advanced actions such as verify, disable, and remove may route to Settings.
- [ ] Add or update Vitest coverage for status-to-action projection if the logic is extracted.
- [ ] Add or update Vitest coverage for the complete settings action matrix.

**Verification**

```powershell
cd .\desktop
pnpm test -- app-types
pnpm build
```

---

## Task 6: Gate Live Fallback On Ready And Enabled

**Files**

- `desktop/src-tauri/src/live/runtime.rs`
- `desktop/src-tauri/src/live/stream.rs`
- `desktop/src-tauri/src/stt/nemotron.rs`
- `desktop/src-tauri/src/stt/error.rs`
- `desktop/src-tauri/src/lib.rs`

**Steps**

- [ ] Add a single Rust helper for "can local fallback start now" so live runtime and setup commands use the same rule.
- [ ] Return `FALLBACK_DISABLED` if the user disabled fallback even when files exist.
- [ ] Return `MODEL_MISSING` for missing artifacts.
- [ ] Return `MODEL_CORRUPT` for corrupt artifacts.
- [ ] Do not call `ensure_model()` from live start. Live start may read marker/status data, but it must not run full SHA verification or download on the hot path.
- [ ] Keep this helper scoped to fallback availability only. It must not choose between `serverLive` and `localFallback`; future route selection still belongs to the client state machine and server connector policy, where server-ready live wins.
- [ ] Update tests around `recordingStatusForStartFailure` only if error codes change.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live
cd .\desktop
pnpm test -- app-types
```

---

## Task 7: Final Verification And Commit

**Files**

- All files touched by the tasks above.

**Steps**

- [ ] Run formatting.
- [ ] Run Rust tests:

```powershell
cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
```

- [ ] Run frontend tests:

```powershell
cd .\desktop
pnpm test
pnpm build
```

- [ ] Confirm no hidden package-manager or model-selector churn:

```powershell
git diff -- desktop\package.json desktop\pnpm-lock.yaml desktop\src-tauri\Cargo.toml desktop\src-tauri\Cargo.lock
```

- [ ] Commit only the model download UX implementation changes:

```powershell
git status --short
git add desktop/src-tauri/src/stt/model.rs desktop/src-tauri/src/stt/nemotron.rs desktop/src-tauri/src/stt/error.rs desktop/src-tauri/src/stt/mod.rs desktop/src-tauri/src/lib.rs desktop/src/settings.ts desktop/src/lib/app-types.ts desktop/src/App.tsx desktop/src/components/panels/app-sheets.tsx
git commit -m "feat: add explicit fallback model lifecycle"
```

**Expected Outcome**

- Settings shows a clear one-model lifecycle surface.
- Live fallback cannot trigger hidden downloads.
- Corrupt files are visible and blocked.
- The user can install, cancel, verify, remove, disable, enable, and open the model folder.
