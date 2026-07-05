# Client State Machine Design

**Status:** Draft
**Date:** 2026-07-05
**Scope:** Phase 1/2 desktop workflow architecture, with hooks for Phase 3-8 server, preprocessing, and diarization.
**Canonical build spec:** [../../specs/client-state-machine.md](../../specs/client-state-machine.md)

## Problem

The desktop currently works, but its state is still UI-shaped:

- `desktop/src/App.tsx` owns queue, setup, running, history, selection, and status text directly.
- `UploadItem` and `UploadStatus` live in `desktop/src/components/stacked-upload.tsx`, even though the queue is app state.
- Rust currently exposes setup status and local STT events, but not a client/runtime job state machine.
- The queue can only say `queued`, `running`, `done`, or `error`, which is not enough for the product direction.

The product direction is no longer "local model does everything." Yap is a thin desktop client with local Moonshine tiny for live/offline fallback, server Cohere for official larger recordings, queued/blocking behavior when the server path is unavailable, and preprocessing/diarization as first-class pipeline stages.

The previous readiness helper was the wrong abstraction. It created side-state around the app instead of turning the app's real workflow into a state machine.

## Decision

Build the client workflow around a typed recording state machine.

Long-term ownership belongs in Tauri Rust as a `RuntimeOrchestrator`, matching ADR 0006. React should project typed runtime/job snapshots and send user intents. Phase 1/2 can temporarily keep the queue projection in React while we move component-owned types into shared app types, but the vocabulary must match the Rust orchestrator target.

Do not add another standalone readiness helper.

Do change the real owners:

- Move queue/domain projection types out of `stacked-upload.tsx` into `desktop/src/lib/app-types.ts`.
- Rename the app-level concept from `UploadItem` to a recording-job view.
- Replace four-value `UploadStatus` with job states that can represent setup, server, local fallback, queueing, preprocessing, diarization, saving, and retryable failure.
- Introduce a Rust `RuntimeOrchestrator` implementation slice after the React projection is untangled.
- Keep UI copy terse. UI renders state labels; docs explain the architecture.

## Required Axes

Use the canonical axes from [client-state-machine.md](../../specs/client-state-machine.md):

- Setup: `checking`, `fallback_missing`, `fallback_installing`, `fallback_ready`, `fallback_disabled`, `setup_error`.
- Server connector: `not_set`, `connecting`, `ready`, `offline`, `sign_in_required`, `retrying`, `disabled`.
- Runtime: `idle`, `fallback_ready`, `fallback_running`, `server_queued`, `server_uploading`, `live_ready`, `live_active`, `background_enriching`, `degraded_background`.
- Job: `accepted`, `preflighting`, blocked states, queued states, local/server processing, preprocessing, diarization, `complete`, `partial`, `failed`, `cancelled`.

## Implementation Shape

| Area | Owner | Purpose |
|------|-------|---------|
| React projection | `desktop/src/App.tsx` and `desktop/src/lib/app-types.ts` | Immediate cleanup of queue state and labels. |
| Runtime orchestration | `desktop/src-tauri/src/runtime/` | Source of truth for runtime transitions and invariants. |
| Local fallback execution | `desktop/src-tauri/src/stt/dispatch.rs` | Executes current Moonshine fallback path and emits job events. |
| UI rendering | `desktop/src/components/stacked-upload.tsx`, `queue-panel.tsx`, `app-sheets.tsx` | Renders typed snapshots without owning domain state. |
| Docs | `docs/specs/client-state-machine.md`, ADR 0006/0014/0015 links | Keeps implementation aligned with server trajectory. |

## Acceptance Criteria

- No standalone readiness helper module exists.
- `UploadItem` no longer leaves `stacked-upload.tsx`; the app uses recording-job workflow types.
- Queue state can represent blocked setup/server/auth, local fallback, server queued/uploading/processing, preprocessing, diarization, saving, complete, partial, failed, and cancelled.
- Preprocessing, alignment, and diarization are represented in the job pipeline model, even if Phase 1/2 mark them as not started or skipped.
- The plan contains a Rust `RuntimeOrchestrator` slice rather than leaving the state machine permanently in React.
