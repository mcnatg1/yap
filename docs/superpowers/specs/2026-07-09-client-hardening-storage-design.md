# Client Hardening And Storage Design

**Status:** Draft
**Date:** 2026-07-09
**Scope:** Remaining desktop maintainability, storage, and native trust-boundary hardening on `hardening/yap-maintainability`.

## Problem

The desktop client is now smaller and easier to inspect, but several seams still need a clear contract before more implementation work:

- `desktop/src/App.tsx` is down to roughly 950 lines, but still owns queue execution, setup/model state, status labels, and history actions.
- `desktop/src-tauri/src/lib.rs` is down to roughly 1,070 lines, but still owns command definitions, setup glue, global-shortcut registration, and the invoke list.
- History uses bounded windowing, not a measured virtual scroller.
- Search can still inspect all bounded history entries and load transcript previews.
- Playback paths are Rust-minted, but the broader import/open/reveal/transcribe path boundary still accepts raw frontend strings.
- Local storage is split across app-data files and bounded frontend metadata, so we need an explicit answer on SQLite.

## Decision

Do not add SQLite in this phase.

Keep actual user artifacts as inspectable files, and treat frontend storage as bounded indexes over those files:

| Data | Current owner | Keep for now | Why |
|------|---------------|--------------|-----|
| Live transcripts and WAVs | `%LOCALAPPDATA%/Yap/live-recordings` via Rust | Yes | User-owned, inspectable files that can be rescanned. |
| Transcript history | `localStorage`, capped at `500` | Yes | A small index over files; no query engine needed yet. |
| Hidden history | `localStorage` | Yes | Tiny preference list. |
| Queued server recordings | `localStorage`, capped at `200` | Yes, temporarily | A queue shell until the server connector defines durable jobs. |
| Playback registry | Rust JSON in app data, capped at `500` | Yes | Rust-minted allowlist is safer than trusting frontend state. |
| Settings/model/live state | Rust app-data files | Yes | Simple, recoverable, and testable. |
| Setup prompt skip flag | `localStorage` in `App.tsx` | Temporary | Move only when setup state gets a focused owner. |

SQLite becomes allowed only after at least one trigger is true:

- History/search exceeds the `500` entry cap or measured search/render time exceeds the UI budget.
- Queue state needs transactional retry/drain across native tasks.
- Server connector creates durable job IDs, retry attempts, upload state, auth state, and per-job metadata.
- We need local cross-entity queries across transcripts, queue, settings, and model state.

Even then, SQLite should be an index/job-state store, not a blob store. Transcript/audio bytes stay as files.

## Architecture Rules

- Desktop remains a thin client with local live/offline fallback.
- Rust owns native trust boundaries: paths, imported media, asset authorization, hotkeys, overlay windows, live runtime, file deletion, and future import IDs.
- React may own UI selection and transient screen state, but should not mint trust decisions.
- Server connector and queued batch drain remain deferred in this spec.
- Local-only fallback/history code must stay separate from future server streaming code.
- Bounded lists are acceptable when the bounds are enforced and tested.
- Cross-app paste/injection is not part of this phase. It should be removed, disabled, or explicitly gated behind a later ADR/spec before it is treated as foundation behavior.

## Storage Boundary

Use existing focused modules as the migration seam:

| Boundary | Current module |
|----------|----------------|
| Transcript history and hidden history | `desktop/src/history.ts` |
| Queued recording shell | `desktop/src/recording-queue.ts` |
| Setup prompt skip flag | `desktop/src/App.tsx` until setup state is split |
| Playback registry | `desktop/src-tauri/src/file_actions.rs` |
| Live recordings and transcripts | `desktop/src-tauri/src/live/recordings.rs` |
| Local fallback settings | `desktop/src-tauri/src/stt/settings.rs` |
| Live settings | `desktop/src-tauri/src/live/settings.rs` |

Do not add a generic storage service. Do not add new direct `localStorage` reads from UI components. If SQLite becomes necessary, replace these module internals first and keep callers stable.

## Import And File Trust Boundary

The current playback registry is a good start, but the final boundary should be stricter:

- Rust should mint or restore an import identity for every external recording that can be opened, revealed, transcribed, asset-authorized, or queued.
- Frontend/localStorage paths should be treated as hints, not authority.
- Yap-owned live transcripts and WAVs can be trusted through the app-data live-recordings directory checks.
- External imports must be validated through the Rust registry before open/reveal/transcribe.
- The asset protocol should authorize only Yap-owned files or Rust-registered imports.
- Corrupt or missing registry state should degrade by asking the user to re-import, not by trusting raw frontend paths.

SQLite is not required to implement this. A versioned Rust JSON registry is enough until import state needs multi-table queries or transactional job state.

## App Orchestration Boundary

`App.tsx` may remain the top-level composition file, but each remaining extraction needs a single owner and tests.

Already extracted:

- Live controls: `desktop/src/hooks/use-live-control.ts`
- Server state: `desktop/src/hooks/use-server-connection.ts`
- Local compute controls: `desktop/src/hooks/use-local-compute-targets.ts`
- Recording drop/selection: `desktop/src/hooks/use-recording-drop.ts`, `desktop/src/hooks/use-recording-selection.ts`
- Transcript history/text/file/preview state: `desktop/src/hooks/use-transcript-history.ts`, `desktop/src/hooks/use-transcript-text.ts`, `desktop/src/hooks/use-transcript-file-actions.ts`, `desktop/src/hooks/use-transcript-preview.ts`
- Workspace navigation: `desktop/src/hooks/use-workspace-navigation.ts`

Remaining App seams:

- Queue execution and retry: stays in App until server connector/job ownership exists.
- Setup/model status: may move to a focused hook only if `loadStatus`, `applyFallbackModelView`, setup prompting, and queue unblocking become explicit testable projections.
- History row actions: may move only if it reduces App state coupling without creating another cache owner.
- Live saved event handling: should remain near history/navigation until there is a shared event projection hook.

Workspace navigation owns `activeRail`, `workspaceView`, `detailsOpen`, `helpOpen`, and rail collapse. App injects side effects for "open details and refresh" and "open polish and update status." The setup prompt uses a separate no-refresh path on purpose.

## Native Command Boundary

`lib.rs` should remain the Tauri bootstrap and command registration file, not a domain logic file.

Already extracted:

- Overlay window code: `desktop/src-tauri/src/live/overlay_window.rs`
- Tray wiring: `desktop/src-tauri/src/tray.rs`
- Live app actions: `desktop/src-tauri/src/live/actions.rs`

Remaining native seams:

- Live commands and hotkey mutations should move into a cohesive live command module when touched.
- Global-shortcut setup can move into a small live hotkey plugin/helper module if it reduces `lib.rs` without hiding registration order.
- Hotkey mutation must be idle-only in Rust, not just disabled in UI.
- Rollback or re-register failure should surface as a native error.
- File open/reveal/transcribe should move toward import IDs or registry-validated paths.

## History Rendering And Search

Keep bounded windowing for now:

- History cap: `500`.
- Render window: `80`.
- "Show older" can reveal more rows by fixed increments.
- Full measured virtualization is deferred until profiling shows current bounds are insufficient.

Search must still have a budget:

- Searching across the full bounded history is acceptable only because the cap is `500`.
- Preview loading during search should remain bounded, cancellable/stale-safe, and resilient to missing files.
- If search becomes visibly slow, add debounce/minimum-query/incremental indexing before adding a virtual scroller or SQLite.

## Accepted Risks

- `localStorage` metadata is bounded but not inspectable like app-data files.
- Corrupt history or queue JSON falls back to an empty list. That is acceptable for metadata, not for transcript/audio files.
- Live transcript history can be rebuilt from saved live session files.
- External queued recordings cannot be reconstructed if WebView storage is cleared; that is acceptable until the real server connector exists.
- Delete/export semantics are split: Rust deletes Yap-owned files; frontend metadata hides/removes entries.
- Registry JSON corruption loses external playback/import convenience, but must not expand file trust.

## SQLite Migration Contract

If a future trigger justifies SQLite, the migration must include:

- Versioned app-data database path.
- Import from existing `history.ts` and `recording-queue.ts` JSON metadata.
- Rescan of `%LOCALAPPDATA%/Yap/live-recordings`.
- No migration of transcript/audio bytes into database rows.
- Corruption fallback and backup behavior.
- Tests proving old JSON/file installs migrate without losing user artifacts.
- A single Rust-owned native trust boundary for imported paths.

## Acceptance

- No SQLite dependency is added in this phase.
- Storage decision has explicit upgrade triggers.
- Storage callers keep using focused modules instead of direct `localStorage`.
- No raw frontend/localStorage path can be opened, revealed, transcribed, or asset-authorized unless Rust minted or restored it, or it is Yap-owned.
- `App.tsx` loses orchestration only where a focused owner exists.
- `lib.rs` continues shrinking by native domain, not speculative architecture.
- History remains bounded, search cost is acknowledged, and helper behavior is tested.
- Server connector and queued batch drain stay out of this spec.

## Review Notes

This follows ADR 0014, ADR 0018, and ADR 0019:

- Desktop remains a thin client with local live/offline fallback.
- Server owns official long-recording processing, routing, diarization, and heavier storage.
- Client-side storage stays local and bounded; Rust-owned files are inspectable, while frontend metadata is disposable and behind focused modules.
