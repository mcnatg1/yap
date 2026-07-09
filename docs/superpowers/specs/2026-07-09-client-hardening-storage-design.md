# Client Hardening And Storage Design

**Status:** Draft
**Date:** 2026-07-09
**Scope:** Remaining desktop maintainability hardening after the current `hardening/yap-maintainability` slices.

## Problem

The client is much cleaner than it was, but three areas still need explicit direction:

- `desktop/src/App.tsx` still coordinates live/settings/history screens.
- `desktop/src-tauri/src/lib.rs` still owns live hotkey command glue.
- History is bounded and paged, but not measured-virtualized.

There is also an open storage question: whether local user data should move to SQLite.

## Decision

Do not add SQLite yet.

Keep the current storage split:

| Data | Current owner | Keep for now | Why |
|------|---------------|--------------|-----|
| Live transcripts and WAVs | `%LOCALAPPDATA%/Yap/live-recordings` via Rust | Yes | Files are inspectable, user-owned, and already protected by path checks. |
| Transcript history | `localStorage`, capped at 500 | Yes | Small index over files; no query engine needed. |
| Hidden history | `localStorage` | Yes | Tiny preference list. |
| Queued server recordings | `localStorage`, capped at 200 | Yes | Temporary offline queue shell until server connector exists. |
| Playback registry | Rust JSON in app data | Yes | Rust-minted allowlist is safer than trusting frontend state. |
| Settings/model state | Rust text / marker files | Yes | Existing files are simple and recoverable. |
| Setup prompt skip flag | `localStorage` in `App.tsx` | Yes | One boolean UI preference; move it only if setup state gets its own hook. |

Add SQLite only when one of these becomes true:

- History cap grows beyond `500` and measured rendering/search is slow.
- Queue state needs transactional drain/retry across concurrent native tasks.
- Server connector creates durable job IDs, retry attempts, and per-job metadata that localStorage cannot safely own.
- We need ad hoc local queries across transcripts, settings, queue, and model state.

Until then, SQLite adds a dependency, migrations, backup semantics, corruption handling, and another state owner without deleting enough code.

## Storage Boundary

Use the existing storage modules as the migration seam:

| Boundary | Current module |
|----------|----------------|
| Transcript history and hidden history | `desktop/src/history.ts` |
| Queued recording shell | `desktop/src/recording-queue.ts` |
| Setup prompt skip flag | `desktop/src/App.tsx` until setup state is split |
| Playback registry | `desktop/src-tauri/src/file_actions.rs` |
| Live recordings and transcripts | `desktop/src-tauri/src/live/recordings.rs` |
| Local fallback settings | `desktop/src-tauri/src/stt/settings.rs` |

Do not add a generic storage service. Do not add new direct `localStorage` reads from UI components; the existing setup skip flag is the only tolerated UI-owned key. If SQLite becomes necessary, replace these module internals first and keep the callers stable.

## Accepted Storage Risks

- `localStorage` metadata is bounded, but not inspectable like app-data files.
- Corrupt history or queue JSON currently falls back to an empty list. That is acceptable for metadata, not for transcript/audio files.
- Live transcript history can be rebuilt from saved live session files; external queued recordings cannot be rebuilt if WebView storage is cleared.
- Delete/export semantics are split: Rust deletes Yap-owned files; hidden history and queue metadata are frontend metadata.

## Architecture Rules

- React can own UI selection and screen state.
- Rust owns native trust boundaries: file paths, playback registry, hotkeys, overlay windows, live runtime.
- Server connector and queued batch drain remain deferred.
- Local-only code must stay separate from future server streaming code.
- Bounded lists are acceptable when the bound is enforced and tested.

## Remaining Work

### App orchestration

`App.tsx` may keep top-level composition, but repeated lifecycle groups should move only when touched:

- Live control/update helpers -> one focused hook.
- Setup/model command state -> defer broad extraction until `loadStatus` is untangled from app status, server labels, setup prompting, and queue unblocking.
- History mutation helpers -> existing `history.ts` / small hook only if it removes code from `App.tsx`.

Do not split by layer just to split. Split when a function group has one owner and one testable boundary.

### Native command glue

`lib.rs` should remain the Tauri bootstrap and command registration file. Move only cohesive native domains:

- Done: overlay window code -> `live/overlay_window.rs`.
- Done: tray wiring -> `tray.rs`.
- Next: hotkey/live command glue -> `live/actions.rs` or `live/commands.rs` when editing that area.

Do not move commands into generic service objects.

### History rendering

Keep bounded windowing for now.

Full measured virtualization is deferred because history is capped at 500 entries and the current UI renders 80 at a time. Add measured virtualization only after a profiler or Playwright trace shows the current cap is not enough.

## Acceptance

- No SQLite dependency is added in this phase.
- Storage decision is documented with explicit upgrade triggers.
- Storage callers keep using focused modules rather than direct `localStorage`.
- `App.tsx` loses orchestration only where a focused owner exists.
- `lib.rs` continues shrinking by native domain, not by speculative architecture.
- History remains bounded and test-covered.
- Server connector and queued batch drain stay out of this spec.

## Review Notes

This follows ADR 0018 and ADR 0019:

- Desktop remains a thin client with local live fallback.
- Server owns official long-recording processing and heavier storage.
- Client-side storage stays local and bounded; Rust-owned files are inspectable, while frontend metadata is disposable and must stay behind focused modules.
