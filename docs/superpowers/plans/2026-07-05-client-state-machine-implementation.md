# Client State Machine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the cosmetic readiness-helper approach with a real client recording workflow state machine across existing React queue state and the future Rust `RuntimeOrchestrator`.

**Architecture:** Phase 1/2 starts by changing current app state owners instead of adding side files: shared TypeScript projection types move to `desktop/src/lib/app-types.ts`, and UI components render recording jobs instead of owning `UploadItem`. The durable source of truth then moves into a Tauri Rust `RuntimeOrchestrator` that wraps current STT dispatch state and later owns server connector, preprocessing, and diarization transitions.

**Tech Stack:** Tauri 2, Rust, React 19, TypeScript, Vitest, Cargo tests, existing CrispASR/Moonshine fallback commands.

---

## File Structure

- Create/maintain `docs/specs/client-state-machine.md`: canonical build spec for state axes and transitions.
- Modify `desktop/src/lib/app-types.ts`: TypeScript projection types and UI label helpers.
- Modify `desktop/src/components/stacked-upload.tsx`: render recording-job views; stop exporting app state types.
- Modify `desktop/src/components/panels/queue-panel.tsx`: accept recording-job views and active-status helpers.
- Modify `desktop/src/components/panels/app-sheets.tsx`: accept typed setup/server labels from state.
- Modify `desktop/src/lib/history-utils.ts`: convert history entries to complete recording-job views.
- Modify `desktop/src/App.tsx`: initialize jobs with pipeline state and map current STT events to job statuses.
- Create `desktop/src-tauri/src/runtime/mod.rs`: Rust runtime module entrypoint.
- Create `desktop/src-tauri/src/runtime/state.rs`: Rust state enums, job IDs, snapshots.
- Create `desktop/src-tauri/src/runtime/orchestrator.rs`: transition methods and invariants.
- Modify `desktop/src-tauri/src/lib.rs`: own a runtime orchestrator alongside existing STT state.
- Modify `desktop/src-tauri/src/stt/dispatch.rs`: keep current fallback execution but prepare for job-id/runtime event handoff.
- Test `desktop/src/lib/app-types.test.ts`: projection helpers.
- Test `desktop/src-tauri/src/runtime/*`: Rust transition-table tests.

## Important Constraints

- Do not create a standalone readiness helper.
- Do not add fake server HTTP/WSS calls.
- Do not add a state-machine library.
- Do not remove punctuation support.
- Do not add local Cohere batch fallback.
- Keep UI copy compact; move explanation to docs.
- Do not touch unrelated dirty files such as `desktop/src-tauri/Cargo.toml` unless the implementation genuinely needs a Rust dependency.

---

### Task 1: Type The React Recording Job Projection

**Files:**
- Modify: `desktop/src/lib/app-types.ts`
- Create: `desktop/src/lib/app-types.test.ts`

- [ ] **Step 1: Write failing projection tests**

Create `desktop/src/lib/app-types.test.ts`:

```ts
import { describe, expect, it } from "vitest";

import {
  createInitialPipelineState,
  setupStateLabel,
  serverConnectionLabel,
} from "./app-types";

describe("client recording workflow projection", () => {
  it("initializes future pipeline stages without running them", () => {
    expect(createInitialPipelineState()).toEqual({
      intake: "queued",
      preprocessing: "notStarted",
      transcription: "notStarted",
      alignment: "notStarted",
      diarization: "notStarted",
      postprocessing: "notStarted",
    });
  });

  it("uses terse setup labels", () => {
    expect(setupStateLabel("checking")).toBe("Checking");
    expect(setupStateLabel("fallback_missing")).toBe("Setup");
    expect(setupStateLabel("fallback_installing")).toBe("Installing");
    expect(setupStateLabel("fallback_ready")).toBe("Ready");
    expect(setupStateLabel("fallback_disabled")).toBe("Disabled");
    expect(setupStateLabel("setup_error")).toBe("Needs attention");
  });

  it("uses terse server labels", () => {
    expect(serverConnectionLabel("not_set")).toBe("Not set");
    expect(serverConnectionLabel("connecting")).toBe("Checking");
    expect(serverConnectionLabel("ready")).toBe("Ready");
    expect(serverConnectionLabel("offline")).toBe("Offline");
    expect(serverConnectionLabel("sign_in_required")).toBe("Sign in");
    expect(serverConnectionLabel("retrying")).toBe("Retrying");
    expect(serverConnectionLabel("disabled")).toBe("Disabled");
  });
});
```

- [ ] **Step 2: Run the failing test**

Run:

```powershell
cd C:\dev\cohere-transcribe-local\desktop
pnpm test -- src/lib/app-types.test.ts
```

Expected: fail because these helpers do not exist yet.

- [ ] **Step 3: Add projection types and helpers**

Add this block to `desktop/src/lib/app-types.ts` near the existing shared type exports:

```ts
export type SetupState =
  | "checking"
  | "fallback_missing"
  | "fallback_installing"
  | "fallback_ready"
  | "fallback_disabled"
  | "setup_error";

export type ServerConnectionState =
  | "not_set"
  | "connecting"
  | "ready"
  | "offline"
  | "sign_in_required"
  | "retrying"
  | "disabled";

export type RecordingJobStatus =
  | "accepted"
  | "preflighting"
  | "blocked_setup_required"
  | "blocked_server_unavailable"
  | "blocked_sign_in_required"
  | "queued_local_fallback"
  | "queued_server"
  | "preprocessing"
  | "uploading"
  | "server_processing_cohere"
  | "local_transcribing"
  | "saving"
  | "diarization_queued"
  | "diarization_running"
  | "complete"
  | "partial"
  | "failed"
  | "cancelled";

export type RecordingIntent = "live" | "recording";
export type RecordingRoute = "localFallback" | "serverBatch" | "serverLive";
export type PipelineStageStatus = "notStarted" | "queued" | "running" | "done" | "error" | "skipped";

export type RecordingPipelineState = {
  intake: PipelineStageStatus;
  preprocessing: PipelineStageStatus;
  transcription: PipelineStageStatus;
  alignment: PipelineStageStatus;
  diarization: PipelineStageStatus;
  postprocessing: PipelineStageStatus;
};

export type RecordingJobView = {
  id: number;
  path: string;
  name: string;
  intent: RecordingIntent;
  status: RecordingJobStatus;
  route?: RecordingRoute;
  output?: string;
  error?: string;
  progressPhase?: string;
  progressPercent?: number;
  progressMessage?: string;
  pipeline: RecordingPipelineState;
};

export function createInitialPipelineState(): RecordingPipelineState {
  return {
    intake: "queued",
    preprocessing: "notStarted",
    transcription: "notStarted",
    alignment: "notStarted",
    diarization: "notStarted",
    postprocessing: "notStarted",
  };
}

export function setupStateLabel(state: SetupState) {
  switch (state) {
    case "checking":
      return "Checking";
    case "fallback_missing":
      return "Setup";
    case "fallback_installing":
      return "Installing";
    case "fallback_ready":
      return "Ready";
    case "fallback_disabled":
      return "Disabled";
    case "setup_error":
      return "Needs attention";
  }
}

export function serverConnectionLabel(state: ServerConnectionState) {
  switch (state) {
    case "not_set":
      return "Not set";
    case "connecting":
      return "Checking";
    case "ready":
      return "Ready";
    case "offline":
      return "Offline";
    case "sign_in_required":
      return "Sign in";
    case "retrying":
      return "Retrying";
    case "disabled":
      return "Disabled";
  }
}
```

- [ ] **Step 4: Run the focused test**

Run:

```powershell
cd C:\dev\cohere-transcribe-local\desktop
pnpm test -- src/lib/app-types.test.ts
```

Expected: pass.

---

### Task 2: Move Queue Types Out Of The Component

**Files:**
- Modify: `desktop/src/components/stacked-upload.tsx`
- Modify: `desktop/src/components/panels/queue-panel.tsx`
- Modify: `desktop/src/lib/history-utils.ts`
- Modify: `desktop/src/App.tsx`

- [ ] **Step 1: Stop exporting app state from `stacked-upload.tsx`**

Delete `UploadStatus` and `UploadItem` from `desktop/src/components/stacked-upload.tsx`. Import:

```ts
import { formatElapsed, type RecordingJobStatus, type RecordingJobView } from "@/lib/app-types";
```

Change props and card item types from `UploadItem` to `RecordingJobView`.

- [ ] **Step 2: Replace row status metadata**

Use `Record<RecordingJobStatus, ...>` metadata with these labels:

```ts
const statusMeta = {
  accepted: { label: "Ready", icon: Clock3, progress: 8, variant: "secondary" as const },
  preflighting: { label: "Checking", icon: Loader2, progress: 12, variant: "outline" as const },
  blocked_setup_required: { label: "Setup", icon: XCircle, progress: 0, variant: "secondary" as const },
  blocked_server_unavailable: { label: "Server", icon: XCircle, progress: 0, variant: "secondary" as const },
  blocked_sign_in_required: { label: "Sign in", icon: XCircle, progress: 0, variant: "secondary" as const },
  queued_local_fallback: { label: "Fallback", icon: Clock3, progress: 16, variant: "secondary" as const },
  queued_server: { label: "Server queued", icon: Clock3, progress: 16, variant: "secondary" as const },
  preprocessing: { label: "Preparing", icon: Loader2, progress: null, variant: "outline" as const },
  uploading: { label: "Uploading", icon: Loader2, progress: null, variant: "outline" as const },
  server_processing_cohere: { label: "Server", icon: Loader2, progress: null, variant: "outline" as const },
  local_transcribing: { label: "Fallback", icon: Loader2, progress: null, variant: "outline" as const },
  saving: { label: "Saving", icon: Loader2, progress: 92, variant: "outline" as const },
  diarization_queued: { label: "Speakers queued", icon: Clock3, progress: 100, variant: "secondary" as const },
  diarization_running: { label: "Speakers", icon: Loader2, progress: null, variant: "outline" as const },
  complete: { label: "Done", icon: CheckCircle2, progress: 100, variant: "default" as const },
  partial: { label: "Partial", icon: CheckCircle2, progress: 100, variant: "secondary" as const },
  failed: { label: "Error", icon: XCircle, progress: 100, variant: "destructive" as const },
  cancelled: { label: "Cancelled", icon: XCircle, progress: 0, variant: "secondary" as const },
};
```

- [ ] **Step 3: Update app imports**

Replace every `UploadItem` import/use in `App.tsx`, `queue-panel.tsx`, and `history-utils.ts` with `RecordingJobView`.

- [ ] **Step 4: Run focused search**

Run:

```powershell
cd C:\dev\cohere-transcribe-local
rg -n "UploadItem|UploadStatus" desktop/src
```

Expected: no results.

---

### Task 3: Map Current STT Events Into The New Job View

**Files:**
- Modify: `desktop/src/App.tsx`
- Modify: `desktop/src/lib/history-utils.ts`

- [ ] **Step 1: Initialize new file jobs**

In `addPaths`, create `RecordingJobView` objects with:

```ts
{
  id: nextId + index,
  intent: "recording",
  name: basename(path),
  path,
  pipeline: createInitialPipelineState(),
  route: "localFallback",
  status: "queued_local_fallback",
}
```

- [ ] **Step 2: Map fallback progress**

In `updateItemProgress`, set:

```ts
status: event.phase === "writing" ? "saving" : "local_transcribing",
route: "localFallback",
pipeline: {
  ...entry.pipeline,
  intake: "done",
  transcription: "running",
},
```

- [ ] **Step 3: Map fallback completion**

On successful file completion, set:

```ts
status: "complete",
pipeline: {
  ...entry.pipeline,
  intake: "done",
  transcription: "done",
  postprocessing: "done",
},
```

On per-file error, set:

```ts
status: "failed",
pipeline: {
  ...entry.pipeline,
  transcription: "error",
},
```

- [ ] **Step 4: Map history entries**

Rename the history adapter to `historyEntryToRecordingJob`; it must return `RecordingJobView` with `status: "complete"` and `pipeline.intake/transcription/postprocessing` set to `done`.

- [ ] **Step 5: Run build**

Run:

```powershell
cd C:\dev\cohere-transcribe-local\desktop
pnpm build
```

Expected: TypeScript and Vite build pass after all old status literals are replaced.

---

### Task 4: Add Rust Runtime Orchestrator State

**Files:**
- Create: `desktop/src-tauri/src/runtime/mod.rs`
- Create: `desktop/src-tauri/src/runtime/state.rs`
- Create: `desktop/src-tauri/src/runtime/orchestrator.rs`
- Modify: `desktop/src-tauri/src/lib.rs`

- [ ] **Step 1: Add Rust state enums**

Create `desktop/src-tauri/src/runtime/state.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupState {
    Checking,
    FallbackMissing,
    FallbackInstalling,
    FallbackReady,
    FallbackDisabled,
    SetupError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerConnectorState {
    NotSet,
    Connecting,
    Ready,
    Offline,
    SignInRequired,
    Retrying,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Idle,
    FallbackReady,
    FallbackRunning,
    ServerQueued,
    ServerUploading,
    LiveReady,
    LiveActive,
    BackgroundEnriching,
    DegradedBackground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobRoute {
    LocalFallback,
    ServerBatch,
    ServerLive,
}
```

- [ ] **Step 2: Add orchestrator skeleton and invariants**

Create `desktop/src-tauri/src/runtime/orchestrator.rs`:

```rust
use super::state::{JobRoute, RuntimeState, ServerConnectorState, SetupState};

#[derive(Debug)]
pub struct RuntimeOrchestrator {
    setup: SetupState,
    server: ServerConnectorState,
    runtime: RuntimeState,
}

impl Default for RuntimeOrchestrator {
    fn default() -> Self {
        Self {
            setup: SetupState::Checking,
            server: ServerConnectorState::NotSet,
            runtime: RuntimeState::Idle,
        }
    }
}

impl RuntimeOrchestrator {
    pub fn setup(&self) -> SetupState {
        self.setup
    }

    pub fn server(&self) -> ServerConnectorState {
        self.server
    }

    pub fn runtime(&self) -> RuntimeState {
        self.runtime
    }

    pub fn set_setup(&mut self, setup: SetupState) {
        self.setup = setup;
        if setup == SetupState::FallbackReady && self.runtime == RuntimeState::Idle {
            self.runtime = RuntimeState::FallbackReady;
        }
    }

    pub fn set_server(&mut self, server: ServerConnectorState) {
        self.server = server;
    }

    pub fn route_recording(&self, larger_recording: bool) -> Result<JobRoute, &'static str> {
        if larger_recording {
            return match self.server {
                ServerConnectorState::Ready => Ok(JobRoute::ServerBatch),
                _ => Err("server_unavailable"),
            };
        }

        match self.setup {
            SetupState::FallbackReady => Ok(JobRoute::LocalFallback),
            SetupState::FallbackDisabled => Err("fallback_disabled"),
            _ => Err("setup_required"),
        }
    }

    pub fn start_fallback(&mut self) -> Result<(), &'static str> {
        if self.setup != SetupState::FallbackReady {
            return Err("setup_required");
        }
        if self.runtime == RuntimeState::LiveActive || self.runtime == RuntimeState::ServerUploading {
            return Err("runtime_busy");
        }
        self.runtime = RuntimeState::FallbackRunning;
        Ok(())
    }

    pub fn finish_active_work(&mut self) {
        self.runtime = match self.setup {
            SetupState::FallbackReady => RuntimeState::FallbackReady,
            _ => RuntimeState::Idle,
        };
    }
}
```

- [ ] **Step 3: Add module entrypoint**

Create `desktop/src-tauri/src/runtime/mod.rs`:

```rust
mod orchestrator;
pub mod state;

pub use orchestrator::RuntimeOrchestrator;
```

- [ ] **Step 4: Wire module into lib**

In `desktop/src-tauri/src/lib.rs`, add:

```rust
mod runtime;
```

Add a `RuntimeOrchestrator` field only after checking existing `AppState` shape, wrapping it in the same synchronization pattern used for STT state. Do not remove existing STT behavior in this task.

- [ ] **Step 5: Add Rust transition tests**

Add tests in `orchestrator.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn larger_recording_requires_server_ready() {
        let mut runtime = RuntimeOrchestrator::default();
        runtime.set_setup(SetupState::FallbackReady);
        assert_eq!(runtime.route_recording(true), Err("server_unavailable"));
        runtime.set_server(ServerConnectorState::Ready);
        assert_eq!(runtime.route_recording(true), Ok(JobRoute::ServerBatch));
    }

    #[test]
    fn fallback_requires_setup_ready() {
        let mut runtime = RuntimeOrchestrator::default();
        assert_eq!(runtime.start_fallback(), Err("setup_required"));
        runtime.set_setup(SetupState::FallbackReady);
        assert_eq!(runtime.start_fallback(), Ok(()));
        assert_eq!(runtime.runtime(), RuntimeState::FallbackRunning);
    }
}
```

- [ ] **Step 6: Run Rust tests**

Run:

```powershell
cd C:\dev\cohere-transcribe-local
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml runtime
```

Expected: new runtime tests pass.

---

### Task 5: Document Runtime-Orchestrator Follow-Up Work

**Files:**
- Modify: `docs/VOICE-OS-ARCHITECTURE.md`
- Modify: `docs/specs/client-state-machine.md`
- Modify: `docs/specs/testing-strategy.md`

- [ ] **Step 1: Ensure state names match ADR 0006**

Search:

```powershell
cd C:\dev\cohere-transcribe-local
$stale = @(("Server" + "Running"), ("running" + "Server"), ("route" + "-aware"), ("route" + "State")) -join "|"
rg -n $stale README.md docs desktop/src
Test-Path ("desktop\src\client" + "-readiness.ts")
```

Expected: `rg` prints no matches and `Test-Path` prints `False`.

- [ ] **Step 2: Add testing-strategy coverage**

Add a small section to `docs/specs/testing-strategy.md`:

```md
## Client State Machine Tests

- Rust transition-table tests cover runtime invariants: live vs batch exclusion, large-recording block when server is offline, fallback setup races, and finish/error transitions.
- Frontend projection tests cover setup/server labels, blocked jobs, retry rows, and history-to-job conversion.
- Future contract tests cover server health/auth, batch upload/job status, live WSS tokens, and fallback events.
- Event-order tests must use job IDs before server upload work ships.
```

- [ ] **Step 3: Run diff hygiene**

Run:

```powershell
cd C:\dev\cohere-transcribe-local
git diff --check
git status --short --branch
```

Expected: no whitespace errors. Status shows only intentional docs/app changes plus pre-existing unrelated dirty files.

## Self-Review

- Spec coverage: the plan covers React projection cleanup, Rust runtime ownership, route policy, setup/server axes, preprocessing/alignment/diarization pipeline hooks, docs, and tests.
- Placeholder scan: there are no TBD/TODO placeholders or "similar to" instructions.
- Type consistency: `RecordingJobView`, `RuntimeOrchestrator`, setup/server/runtime states, and server queued/uploading terminology are used consistently.
