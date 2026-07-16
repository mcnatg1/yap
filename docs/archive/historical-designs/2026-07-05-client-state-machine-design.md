# Client State Machine Design

**Status:** Historical design snapshot; superseded as current-state authority
**Date:** 2026-07-05
**Scope:** Phase 1/2 desktop workflow architecture, with hooks for Phase 3-8 server, preprocessing, and diarization.
**Canonical build spec:** [../../specs/client-state-machine.md](../../specs/client-state-machine.md)

> **Archive notice (2026-07-15):** Status statements below describe the system
> at earlier checkpoints. Use [current architecture](../../architecture/CURRENT-ARCHITECTURE.md)
> and [current status](../../CURRENT-STATUS.md) for the executing Phase 1–5
> system. In particular, the speculative umbrella `RuntimeOrchestrator` was
> later removed in favor of explicit connector, job, live, and lifecycle owners.

> **Current truth (2026-07-12):** The statements below describe the pre-implementation starting point. Shared recording-job types and a Rust `RuntimeOrchestrator` skeleton now exist, while imported jobs still use a numeric React/localStorage queue. Durable Rust/SQLite ownership remains in the Phase 3 connector plan.

## Problem

At the time of this design, the desktop worked but its state was still UI-shaped:

- `desktop/src/App.tsx` owns queue, setup, running, history, selection, and status text directly.
- `UploadItem` and `UploadStatus` live in `desktop/src/components/stacked-upload.tsx`, even though the queue is app state.
- Rust exposed setup status and local STT events, but not a client/runtime job state machine; the skeleton has since landed.
- The queue can only say `queued`, `running`, `done`, or `error`, which is not enough for the product direction.

The product direction is no longer "local model does everything." Yap is a thin desktop client with local Nemotron INT8 for live/offline fallback, a server router for official larger recordings, queued/blocking behavior when the server path is unavailable, and preprocessing/diarization as first-class pipeline stages.

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
| Local fallback execution | `desktop/src-tauri/src/live/runtime.rs`, `desktop/src-tauri/src/live/stream.rs`, `desktop/src-tauri/src/stt/nemotron.rs` | Captures mic audio, streams it through the warm Nemotron fallback, and emits live-session snapshots. |
| UI rendering | `desktop/src/components/stacked-upload.tsx`, `queue-panel.tsx`, `app-sheets.tsx` | Renders typed snapshots without owning domain state. |
| Docs | `docs/specs/client-state-machine.md`, ADR 0006/0014/0015 links | Keeps implementation aligned with server trajectory. |

## Acceptance Criteria

- No standalone readiness helper module exists.
- `UploadItem` no longer leaves `stacked-upload.tsx`; the app uses recording-job workflow types.
- Queue state can represent blocked setup/server/auth, local fallback, server queued/uploading/processing, preprocessing, diarization, saving, complete, partial, failed, and cancelled.
- Preprocessing, alignment, and diarization are represented in the job pipeline model, even if Phase 1/2 mark them as not started or skipped.
- The plan contains a Rust `RuntimeOrchestrator` slice rather than leaving the state machine permanently in React.
