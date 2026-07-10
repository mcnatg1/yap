# Spec: Model Download UX

**Status:** Implemented baseline; artifact licensing, native CI, and release verification remain
**Scope:** User-facing lifecycle for Yap's single pinned local fallback model.

This spec improves setup/settings UX for the local Nemotron fallback without adding a local model router. The desktop still owns exactly one local audio inference path: Nemotron 3.5 ASR Streaming 0.6B INT8 through `sherpa-onnx`. Server-side model routing, batch ASR, diarization, and fusion stay out of the desktop client.

## Product Rule

Yap should make local model ownership obvious:

- Runtime never silently downloads model files.
- Setup and settings expose explicit install, cancel, verify, remove, disable, and open-folder actions.
- The UI uses short labels; docs and tooltips carry detail.
- Corrupt or partial installs fail closed.
- Removing the model disables local fallback until the user reinstalls it.

## Meetily Reference

Use Meetily as a reference for lifecycle coverage, not architecture. Their useful pieces are:

| Meetily file | Useful reference | Why it matters |
|--------------|------------------|----------------|
| `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs:28` | `ModelStatus` | Covers missing, downloading, error, corrupt, available states. |
| `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs:38` | `DownloadProgress` | Reports bytes, percent, total MB, and speed. |
| `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs:73` | `ModelInfo` | Central status object for UI projection. |
| `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs:167` | `discover_models` | Separates status discovery from loading. |
| `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs:543` | `download_model_detailed` | Shows resumable download/progress/cancel patterns. |
| `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs:1051` | `cancel_download` | Cancels an active download and cleans state. |
| `frontend/src-tauri/src/parakeet_engine/commands.rs:379` | `parakeet_download_model` | Emits progress events to the frontend. |
| `frontend/src-tauri/src/parakeet_engine/commands.rs:467` | `parakeet_cancel_download` | Frontend command shape for cancellation. |
| `frontend/src-tauri/src/parakeet_engine/commands.rs:542` | `parakeet_delete_corrupted_model` | User recovery for bad files. |
| `frontend/src-tauri/src/parakeet_engine/commands.rs:560` | `open_parakeet_models_folder` | Opens the cache folder for inspection. |
| `frontend/src/lib/parakeet.ts` | Frontend invoke wrapper | Keeps UI calls typed and boring. |
| `frontend/src/components/ParakeetModelManager.tsx` | Settings surface | Useful control grouping; do not copy model selection. |

Do not copy Meetily's provider selector, Whisper support, Parakeet model list, or client-side GPU routing.

## Yap Current Anchors

| Yap file | Current responsibility | Expected change |
|----------|------------------------|-----------------|
| `desktop/src-tauri/src/stt/model.rs` | Model directory, SHA-256, Hugging Face URL, blocking download. | Add progress/cancel-friendly primitives only if current functions are too coarse. |
| `desktop/src-tauri/src/stt/nemotron.rs` | Pinned artifact list, install, verify, remove. | Become the single source of fallback model status. |
| `desktop/src-tauri/src/stt/error.rs` | Stable STT error codes and user messages. | Add only lifecycle errors the UI can act on. |
| `desktop/src-tauri/src/lib.rs` | Tauri command wiring. | Expose small commands; move implementation to `stt/` modules. |
| `desktop/src/settings.ts` | Frontend settings invokes. | Add typed model lifecycle invokes/events. |
| `desktop/src/components/panels/app-sheets.tsx` | Settings dialog and model controls. | Replace generic busy state with lifecycle rows/actions. |
| `desktop/src/lib/app-types.ts` | Setup labels and state projection. | Keep labels aligned with the lifecycle states below. |

## Lifecycle Model

Use one local status object:

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
  label: "Nemotron local fallback";
  status: FallbackModelStatus;
  installedBytes?: number;
  totalBytes?: number;
  progressPercent?: number;
  speedMbps?: number;
  message?: string;
  modelsDir: string;
};
```

Rust may use snake_case internally, but serialized values must match the TypeScript union.

## Commands And Events

Commands:

| Command | Result |
|---------|--------|
| `fallback_model_status()` | Returns `FallbackModelView`. |
| `fallback_model_install()` | Starts explicit install and returns immediately after scheduling or runs synchronously with progress events. |
| `fallback_model_cancel_install()` | Cancels an active install and leaves status `missing` or `error`. |
| `fallback_model_verify()` | Rehashes files, repairs verified markers, returns status. |
| `fallback_model_remove()` | Deletes model artifacts and verified markers. |
| `fallback_model_set_enabled(enabled)` | Persists enabled/disabled state. |
| `fallback_model_open_folder()` | Opens the model cache folder. |

Events:

| Event | Payload |
|-------|---------|
| `fallback-model-status` | `FallbackModelView` after status changes. |
| `fallback-model-progress` | `FallbackModelView` during download/verify. |

The install command can remain simple. If cancellation is not safe with the current blocking `reqwest` path, the first implementation may disable cancel during the file write and mark that in UI copy. Add true cancellation when the download loop is made chunked.

## UI Behavior

Settings should show one compact model section:

| State | Primary action | Secondary action | Copy |
|-------|----------------|------------------|------|
| `missing` | Install | Open folder | `Local fallback is not installed.` |
| `downloading` | Cancel | Open folder | `Downloading 42% · 12 MB/s` |
| `verifying` | None | Open folder | `Verifying files` |
| `ready` | Reinstall | Remove | `Ready` |
| `corrupted` | Repair | Remove | `Files failed verification.` |
| `disabled` | Enable | Remove | `Disabled` |
| `error` | Retry | Remove | Last actionable error. |

Use existing UI primitives. Do not add a package. Destructive removal uses the existing alert dialog component.

## Failure Policy

- If any artifact hash fails, status is `corrupted`.
- If any artifact is missing, status is `missing`.
- If install fails mid-file, leave partial files resumable only if the downloader can prove byte ranges are safe; otherwise delete the partial file.
- If verification fails after download, delete the `.verified` marker and report `corrupted`.
- If the model is disabled, live fallback is blocked even when files exist.

## Out Of Scope

- No Whisper, Parakeet, Moonshine, or Cohere local model selector.
- No client-side GPU routing.
- No background auto-download.
- No server model management.
- No package-manager or monorepo tooling changes.

## Acceptance

- Settings can show missing, downloading, verifying, ready, corrupted, disabled, and error states.
- The user can install, remove, disable, enable, verify, and open the model folder.
- Progress events are visible during install or verification when those operations take noticeable time.
- Runtime live fallback refuses to start unless status is `ready` and enabled.
- Corrupt artifacts never load.
- Existing checks still pass: `pnpm test`, `pnpm build`, and `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml`.
