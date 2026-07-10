# Client Hardening And Storage Design

**Status:** Reviewed
**Date:** 2026-07-09
**Scope:** Remaining desktop maintainability, storage, and native trust-boundary hardening on `hardening/yap-maintainability`.

## Problem

The desktop client is now smaller and easier to inspect, but several seams still need a clear contract before more implementation work:

- `desktop/src/App.tsx` is down to roughly 950 lines, but still owns queue execution, setup/model state, status labels, and history actions.
- `desktop/src-tauri/src/lib.rs` is down to roughly 1,070 lines, but still owns command definitions, setup glue, global-shortcut registration, and the invoke list.
- History uses bounded windowing, not a measured virtual scroller.
- Search can still inspect all bounded history entries and load transcript previews.
- Playback paths are Rust-minted, but the broader import/open/reveal/transcribe path boundary still accepts raw frontend strings.
- Local storage is split across app-data files, bounded frontend projections, and artifact-correlated hidden tombstones, so we need an explicit answer on SQLite.

## Decision

Do not add SQLite in this phase.

Keep actual user artifacts as inspectable files. Treat frontend storage as bounded disposable projections plus artifact-correlated hidden tombstones:

| Data | Current owner | Keep for now | Why |
|------|---------------|--------------|-----|
| Live transcripts and WAVs | Effective Rust live-recordings directory (app data or explicit override) | Yes | User-owned, inspectable files whose canonical primary sessions can be rescanned. |
| Transcript history | `localStorage`, capped at `500` | Yes | A small index over files; no query engine needed yet. |
| Hidden history | `localStorage` | Yes | Tombstones prevent rescanned artifacts from reappearing. |
| Queued server recordings | `localStorage`, capped at `200` | Yes, temporarily | A queue shell until the server connector defines durable jobs. |
| Playback registry | Rust JSON in app data, capped at `500` | Yes | Rust-minted allowlist is safer than trusting frontend state. |
| Preferences/compute/model state | Rust JSON, scalar/marker files, and verified artifacts | Yes | Each has a small explicit contract; current-session state remains memory-only. |
| Setup prompt skip flag | `localStorage` in `App.tsx` | Temporary | Move only when setup state gets a focused owner. |

SQLite becomes appropriate only at the product and measurement gates defined below. Even then, it is an index/job-state store, not a blob or credential store. Transcript/audio bytes stay as files, and authentication secrets stay in OS credential storage.

## User Data Contract

Treat each class of user data according to its actual durability and query needs. "User data" is not one storage problem.

| Data class | Source of truth now | Durability and recovery | Bound | SQLite decision |
|------------|---------------------|-------------------------|-------|-----------------|
| Live WAV/TXT artifacts | Rust-owned files under the effective live-recordings directory, including `YAP_LIVE_RECORDINGS_DIR` overrides | Atomic final-name writes; primary Yap-owned live identity/path/timestamp can be rescanned; warnings, hidden tombstones, and external/server history are not reconstructable | Audio cap enforced by live runtime; retention policy remains explicit user control | Never store bytes in SQLite |
| Live/session metadata | Current live state plus saved artifact names | Locale, language, country code, device/session ID, route, processing, and privacy fields belong to the future server manifest/job contract. Persist non-derivable local metadata in versioned sidecars only when that contract lands | One small record per session/chunk | Use SQLite only when durable jobs need indexed transitions |
| Home history index | `history.ts` in `localStorage` plus Rust rescan of primary live files | Disposable 500-entry projection; malformed JSON becomes an empty index and primary live files partially rebuild | `500` entries; current search reads bounded 600-character previews | Keep now; re-evaluate for retention/search above 500, authoritative non-file metadata, or a measured p95 cold-reconciliation/search budget breach |
| Hidden-history tombstones | `history.ts` in `localStorage` | Prevent still-existing artifacts from reappearing after rescan; prune tombstones only after the artifact is confirmed missing | One tombstone per hidden artifact; not bounded by the visible-history cap | Keep now; co-migrate only with the history index |
| Pending server batch jobs | `recording-queue.ts` in `localStorage` | Temporary shell only; must become a Rust-owned durable ledger before automatic upload/drain ships | `200` entries | SQLite is preferred once jobs have attempts, cancellation, upload offsets, idempotency keys, or reconnect drain |
| External import/playback trust | Rust registry JSON | Rust validates canonical paths; add serialized mutation and version rejection; corrupt registry asks for re-import and never broadens access | `500` entries | Keep JSON now; migrate if stable import IDs participate in jobs/history queries |
| Live preferences and shortcuts | Rust-owned JSON | Atomic replace; add schema versioning and backward-compatible defaults before changing its shape | Small fixed record | Keep JSON |
| Compute target and fallback enablement | Rust-owned scalar file and marker file | Explicit defaults; model readiness is derived from verified model artifacts | Small fixed state | Keep files |
| UI-only selection and open state | React memory | Intentionally ephemeral | One active workspace/session | Never persist in SQLite |

The frontend may cache projections, but Rust remains authoritative for file trust, durable artifacts, native settings, and future job state. No React component may query SQLite directly if it is introduced later; callers continue through typed Tauri commands and the existing focused modules.

## SQLite Decision Gate

The current answer is **not yet**. Re-evaluate at the first of these product gates:

1. **Before real server queue drain:** jobs require durable state transitions, retry counts, cancellation, reconnect recovery, and idempotency. Prefer a Rust-owned SQLite ledger at that point.
2. **Before full-body search/retention above 500:** benchmark the current bounded 600-character preview search. If the product needs larger retention or full-body search, use a Rust-owned rebuildable/contentless FTS index whose tokens follow transcript privacy, deletion, and backup rules.
3. **Before import identities join queue/history state:** migrate the JSON trust registry only if transactional relationships are needed across imports and jobs.
4. **Apply the right trigger:** performance-driven history migration requires a measured budget breach. Correctness-driven job-ledger migration is mandatory before real upload, drain, retry, or background processing ships. Loss of explicitly disposable projection metadata remains an accepted risk unless the product makes it authoritative.

SQLite is explicitly the wrong tool for WAV/Opus bytes, transcript files, model artifacts, credentials, clipboard contents, transient live tokens, shortcuts/settings, compute flags, or overlay state.

## Architecture Rules

- Desktop remains a thin client with local live/offline fallback.
- Rust owns native trust boundaries: paths, imported media, asset authorization, hotkeys, overlay windows, live runtime, file deletion, and future import IDs.
- React may own UI selection and transient screen state, but should not mint trust decisions.
- Server connector and queued batch drain remain deferred in this spec.
- Local-only fallback/history code must stay separate from future server streaming code.
- Bounded lists are acceptable when the bounds are enforced and tested.
- Focused-field injection is an existing client feature governed by ADR 0013 and must be preserved. Yap captures the stop-time external foreground window and the focused child control when Windows exposes it, revalidates the available target data after final decoding, then uses Windows Unicode `SendInput`. A focus change, held modifier, or OS block falls back to a valid-owner clipboard write and visible manual-paste status. The configurable paste-last shortcut targets the then-focused external control and repeats only the last completed transcript.
- Start, stop, and stream-crash transitions share one runtime transition gate, while normal stop and stream-crash completion share one atomic finalizer. Audio workers remain concurrent, but session state, runtime ownership, and orchestrator changes cannot interleave. A crash may still inject a completed final transcript; crash, injection-fallback, and save feedback compose instead of replacing one another, and only one owner may consume the session PCM.

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

- Rust should mint or restore an import identity before an external recording crosses a native open, reveal, transcribe, asset, or future upload/dispatch boundary. The temporary frontend queue may retain a raw path as metadata, never as authority.
- Frontend/localStorage paths should be treated as hints, not authority.
- Yap-owned live transcripts and WAVs can be trusted through the app-data live-recordings directory checks.
- External imports must be validated through the Rust registry before open/reveal/transcribe.
- Registry mutation must be serialized in Rust, unsupported versions must be rejected, and entries referenced by current queue/history state must not be evicted. At capacity the registry fails closed; a deliberate unregister/reclamation UX remains required before the cap becomes reachable product behavior.
- The asset protocol should authorize only Yap-owned files or Rust-registered imports.
- Missing or malformed registry state fails closed as unavailable playback; reselecting/reimporting a recording rebuilds valid state. It must never recover by trusting raw frontend paths.
- Rust emits stable canonical paths for Yap-owned live sessions and resolves existing legacy tombstone candidates to that canonical path. Frontend history uses an OS-aware comparison identity for Windows case, separators, root-clamped dot segments, and verbatim prefixes; stale alias metadata is removed before the tombstone is rewritten, while ordinary display paths remain unchanged.

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

- Visible history and queue projections in `localStorage` are bounded but not inspectable like app-data files; hidden-history tombstones are intentionally not capped by the visible-history limit.
- Corrupt history or queue JSON falls back to an empty list. That is acceptable only while these remain bounded projections/shells, not authoritative transcript/audio/job state.
- Live history can partially rebuild primary `live-<timestamp>[-suffix].txt` artifacts; warning/degradation metadata, tombstones, polished variants, and external/server history require their own durable records.
- External queued recordings cannot be reconstructed if WebView storage is cleared; that is acceptable until the real server connector exists.
- Delete/export semantics are split: Rust deletes Yap-owned files; frontend metadata hides/removes entries.
- Registry JSON corruption loses external playback/import convenience, but must not expand file trust.

## SQLite Migration Contract

If a future trigger justifies SQLite, the migration must include:

- Versioned app-data database path.
- On first DB-capable launch, the focused frontend modules asynchronously hydrate and send versioned history, tombstone, and queue payloads to one idempotent Rust transaction; Rust cannot directly read WebView `localStorage`.
- Validate import identities/paths, commit a migration marker, then retire old frontend keys only after acknowledgement.
- Domain types and module ownership remain stable, but storage APIs may become asynchronous. No history write or queue drain starts before hydration succeeds; failed migration is visible/retryable, and replay after interruption is idempotent.
- Rescan the effective Rust live-recordings directory, including configured overrides, and include only primary `live-<timestamp>[-suffix].txt` artifacts.
- No migration of transcript/audio bytes into database rows.
- Corruption fallback and backup behavior.
- Tests proving old JSON/file installs migrate without losing user artifacts.
- A single Rust-owned native trust boundary for imported paths.
- No direct SQL or database path exposure to the WebView.
- A transactionally updated schema version and rollback-safe migration backup.
- WAL and busy-timeout behavior defined before concurrent background drain is enabled.
- A rebuild command that can recreate history/search rows from app-owned files without trusting frontend paths.

## Acceptance

- No SQLite dependency is added in this phase.
- The data-class contract identifies which stores are authoritative, rebuildable, bounded, or temporary.
- Real server queue drain cannot ship on the current `localStorage` queue shell.
- Storage decision has explicit upgrade triggers.
- Storage callers keep using focused modules instead of direct `localStorage`.
- A raw frontend/localStorage path is metadata, not authorization. It may remain in the bounded queue shell, but it cannot cross native open, reveal, transcribe, asset, or future upload boundaries unless Rust minted or restored it, or it is Yap-owned.
- `App.tsx` loses orchestration only where a focused owner exists.
- `lib.rs` continues shrinking by native domain, not speculative architecture.
- History remains bounded, search cost is acknowledged, and helper behavior is tested.
- Server connector and queued batch drain stay out of this spec.
- On Windows, completion injects the normalized transcript only after the stop-time foreground target is revalidated; otherwise the full transcript is copied and the overlay surfaces manual-paste status.
- The configured paste-last shortcut persists across restart. Its last-completed transcript payload is process-memory-only; during a process lifetime it uses only the last completed transcript, never an active partial, and remains independent of server/database availability.
- Rust filesystem authorization, not frontend path checks, permits tombstone pruning. Existing hidden artifacts remain hidden; only confirmed-missing primary Yap-owned artifacts may lose tombstones, and stale history metadata is removed before its tombstone.
- Registry tests cover unsupported versions, capacity without eviction, and concurrent successful registration. A deliberate capacity-reclamation UX remains a follow-on requirement.

## Review Notes

This follows ADR 0014, ADR 0018, and ADR 0019:

- Desktop remains a thin client with local live/offline fallback.
- Server owns official long-recording processing, routing, diarization, and heavier storage.
- Client-side storage stays local; Rust-owned files are inspectable, bounded frontend projections are disposable, and artifact-correlated hidden tombstones remain behind the focused history module until Rust-authorized reconciliation can remove them.
