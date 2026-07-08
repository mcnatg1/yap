# Task 2 Report: Add chunked download progress and cancellation

## Scope completed

- Backend Rust only.
- Updated:
  - `desktop/src-tauri/src/stt/model.rs`
  - `desktop/src-tauri/src/stt/nemotron.rs`
  - `desktop/src-tauri/src/stt/error.rs`
- No Tauri commands added.
- No frontend or server changes made.

## What changed

### `desktop/src-tauri/src/stt/model.rs`

- Added `DownloadProgress` with:
  - `downloaded_bytes`
  - `total_bytes`
  - `elapsed_ms`
- Added pure progress math through:
  - `DownloadProgress::percent()`
  - `DownloadProgress::speed_mbps()`
  - internal helper `progress_metrics(...)`
- Kept legacy `download_file(url, dest)` as a wrapper over:
  - `download_file_with_progress(url, dest, on_progress, is_cancelled)`
- Replaced `std::io::copy` with a manual 64 KiB buffered read/write loop.
- Reads `content_length()` when available.
- Emits progress:
  - after each successful write
  - once more after the final atomic rename
- Downloads to `*.part` and renames to the final destination on success.
- On cancellation:
  - removes the `.part` file
  - returns `SttError::ModelInstallCancelled`
- On mid-download read/write/rename failure:
  - removes the `.part` file
  - returns the existing install failure surface (`ModelMissing`) for non-cancel failures

### `desktop/src-tauri/src/stt/nemotron.rs`

- Added `ensure_model_with_progress(force, on_progress, is_cancelled)`.
- Kept `ensure_model()` as a wrapper using:
  - `force = false`
  - no-op progress callback
  - never-cancel callback
- Updated artifact installation flow so `ensure_artifact(...)` now:
  - uses `download_file_with_progress(...)`
  - maps file download progress into `FallbackModelView`
  - emits `Downloading` updates during transfer
  - emits a `Verifying` update before sha verification
- Added `force` reinstall behavior:
  - removes stale destination file
  - removes `.verified`
  - removes `.part`
  - then redownloads
- Added cleanup helper `remove_download_artifacts(...)`.
- On download or verification failure:
  - removes destination file / marker / partial file
  - leaves no `.verified` marker behind
  - leaves the artifact state effectively `missing` instead of preserving a bad partial install

### `desktop/src-tauri/src/stt/error.rs`

- Added `SttError::ModelInstallCancelled`
  - code: `MODEL_INSTALL_CANCELLED`
  - message: `Local fallback install was cancelled.`

## Tests added/updated

### `model.rs`

- Progress math coverage:
  - percent calculation
  - speed calculation
  - missing total
  - zero elapsed
  - percent clamping at 100
- Streaming helper coverage without network access:
  - emits per-chunk and final post-rename progress
  - cleans partial file on cancellation
  - cleans partial file on read failure

### `nemotron.rs`

- Cleanup helper coverage:
  - removes artifact file
  - removes `.verified`
  - removes `.part`

### `error.rs`

- Stable code/message coverage updated for the new cancellation variant

## Verification run

Command executed exactly as required:

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml model
```

Observed result:

- 13 tests passed
- 0 failed

## Notes / constraints kept

- Status classification remains read-only.
- No hashing logic was added to status classification.
- Single Nemotron fallback only.
- No UI, no server, and no Tauri command wiring added in this task.

## Follow-on concerns

- `ensure_model_with_progress(...)` currently reports progress per artifact, not as a model-wide aggregate across all four files. That matches the current backend-only scope but may matter when Task 3 wires this into UI.
- Non-cancel network/write/rename failures still collapse to `ModelMissing`, preserving the existing error surface outside the new explicit cancellation path.
