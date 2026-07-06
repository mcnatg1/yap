# Local Moonshine Live Transcription Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the existing live overlay actually transcribe live microphone audio through the pinned local Moonshine v2 tiny fallback.

**Spec:** [../specs/2026-07-05-local-moonshine-live-transcription.md](../specs/2026-07-05-local-moonshine-live-transcription.md)

**Architecture:** Keep React as a view layer. Tauri Rust owns route checks, mic capture, the local stream child, and live-session events. This branch uses a session-bound `crispasr --stream-json` stdio child for local fallback; stop retires the child until CrispASR exposes a reset/ack boundary for safe warm reuse. Server WSS, Opus, Rust Silero inference, save-audio, and diarization remain separate phases.

**Tech Stack:** Tauri 2, Rust 2021, `cpal` already installed, existing `serde_json`, existing pinned CrispASR/Moonshine setup, React 19 live overlay.

---

## File Structure

- Modify `docs/specs/phase-3-live-ux-audio.md`: add Phase 3a amendment that this branch ships local live text but not full Silero/save-audio completion.
- Modify `desktop/src-tauri/src/live/mod.rs`: export new modules.
- Create `desktop/src-tauri/src/live/stream.rs`: CrispASR stream command builder, stream event parser, child launch helpers.
- Create `desktop/src-tauri/src/live/runtime.rs`: `LiveRuntime`, CPAL capture, mono/resample/PCM conversion, session tokens, session-bound child lifecycle.
- Modify `desktop/src-tauri/src/live/state.rs`: preserve final text on stop, add helpers for partial/final/level/error updates.
- Modify `desktop/src-tauri/src/lib.rs`: manage `LiveRuntime`, wire start/stop with `SttState` and `RuntimeOrchestratorState`, run idle cleanup.
- Modify `desktop/src/components/live/live-overlay.tsx`: show final text plus partial text instead of one truncated line.

---

## Constraints

- Do not add new dependencies unless the Rust compiler proves the existing stack cannot do the job.
- Do not reintroduce local Cohere.
- Do not add a fake server connector.
- Do not use `--no-punctuation`.
- Do not do model inference or filesystem work in the CPAL callback.
- Do not erase `finalText` on stop/crash.

---

### Task 1: Mark This As Phase 3a In Docs

**Files:**
- Modify `docs/specs/phase-3-live-ux-audio.md`

- [ ] **Step 1: Add the amendment under the existing 2026-07-05 amendment**

Add this paragraph:

```markdown
> **2026-07-05 Phase 3a amendment:** The local Moonshine live-transcription branch implements real local fallback text streaming through the existing overlay and hotkey surface. It is not full Phase 3 completion: Rust Silero ONNX, `vad_segments` chunk manifests, Opus/server WSS, saved live audio, Scribe, and diarization remain follow-on work. See [Local Moonshine Live Transcription](../superpowers/specs/2026-07-05-local-moonshine-live-transcription.md).
```

- [ ] **Step 2: Verify docs references**

Run:

```powershell
rg -n "Phase 3a|Local Moonshine Live Transcription|vad_segments" docs/specs/phase-3-live-ux-audio.md docs/superpowers/specs/2026-07-05-local-moonshine-live-transcription.md
```

Expected: both files mention Phase 3a and preserve `vad_segments` as follow-on work.

- [ ] **Step 3: Commit**

```powershell
git add docs/specs/phase-3-live-ux-audio.md docs/superpowers/specs/2026-07-05-local-moonshine-live-transcription.md docs/superpowers/plans/2026-07-05-local-moonshine-live-transcription.md
git commit -m "docs: specify local live transcription bridge"
```

---

### Task 2: Add CrispASR Stream Command And Parser

**Files:**
- Create `desktop/src-tauri/src/live/stream.rs`
- Modify `desktop/src-tauri/src/live/mod.rs`

- [ ] **Step 1: Write tests in `stream.rs`**

Add tests for:

```rust
#[test]
fn stream_args_keep_punctuation_and_gpu_choice() {
    let gpu = crate::stt::gpu::GpuStatus {
        available: true,
        adapter_name: Some("test gpu".into()),
        preference: crate::stt::gpu::GpuPreference::Auto,
        layers: 99,
    };
    let args = build_stream_args(
        std::path::Path::new("C:/models/moonshine.gguf"),
        std::path::Path::new("C:/models/punc.gguf"),
        &gpu,
    );
    assert!(args.contains(&"--stream".to_string()));
    assert!(args.contains(&"--stream-json".to_string()));
    assert!(args.contains(&"--punc-model".to_string()));
    assert!(!args.contains(&"--no-punctuation".to_string()));
    assert!(args.contains(&"--gpu-backend".to_string()));
}

#[test]
fn parses_partial_and_final_events() {
    assert_eq!(
        parse_stream_event(r#"{"type":"partial","text":"hello"}"#),
        Some(StreamEvent::Partial("hello".into()))
    );
    assert_eq!(
        parse_stream_event(r#"{"event":"final","text":"hello."}"#),
        Some(StreamEvent::Final("hello.".into()))
    );
    assert_eq!(parse_stream_event("not json"), None);
}
```

- [ ] **Step 2: Run tests and see failure**

```powershell
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml live::stream
```

Expected: compile fails because the module/functions do not exist yet.

- [ ] **Step 3: Implement minimal stream module**

Implement:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Partial(String),
    Final(String),
}

pub fn build_stream_args(model: &Path, punc_model: &Path, gpu: &crate::stt::gpu::GpuStatus) -> Vec<String> {
    let mut args = vec![
        "--stream".into(),
        "--stream-json".into(),
        "--backend".into(),
        "moonshine-streaming".into(),
        "-m".into(),
        model.to_string_lossy().to_string(),
        "-l".into(),
        "en".into(),
        "--punc-model".into(),
        punc_model.to_string_lossy().to_string(),
    ];
    if gpu.layers > 0 {
        args.push("--gpu-backend".into());
        args.push("auto".into());
    } else {
        args.push("-ng".into());
    }
    args
}
```

`parse_stream_event` must parse JSON, read `text`, inspect `type`/`event`/`status`, return `Final` when the kind contains `final`, `Partial` when it contains `partial`, and treat non-empty untyped text as `Partial`.

- [ ] **Step 4: Add child spawn helpers**

Add helpers that resolve the pinned binary/model/punctuation paths using existing `stt::binary`, `stt::pin`, and `stt::model` functions. Resolve GPU routing with `crate::stt::gpu::GpuStatus::resolve()` and pass that status to `build_stream_args`. Spawn with piped stdin/stdout, null stderr or existing Yap log path, and hidden console.

- [ ] **Step 5: Run focused tests**

```powershell
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml live::stream
```

Expected: stream module tests pass.

- [ ] **Step 6: Commit**

```powershell
git add desktop/src-tauri/src/live/mod.rs desktop/src-tauri/src/live/stream.rs
git commit -m "feat: add local live stream command"
```

---

### Task 3: Add LiveRuntime Audio Bridge

**Files:**
- Create `desktop/src-tauri/src/live/runtime.rs`
- Modify `desktop/src-tauri/src/live/mod.rs`
- Modify `desktop/src-tauri/src/live/state.rs`

- [ ] **Step 1: Write tests**

Add unit tests for:

```rust
#[test]
fn mono_downmix_averages_channels() {
    assert_eq!(downmix_to_mono(&[1.0, 3.0, 2.0, 4.0], 2), vec![2.0, 3.0]);
}

#[test]
fn pcm_conversion_clamps_to_i16() {
    assert_eq!(f32_to_i16_le_bytes(&[-2.0, 0.0, 2.0]), vec![0, 128, 0, 0, 255, 127]);
}

#[test]
fn linear_resample_can_downsample() {
    let mut resampler = LinearResampler::new(4, 2);
    assert_eq!(resampler.push(&[0.0, 1.0, 0.0, -1.0]), vec![0.0, 0.0]);
}

#[test]
fn stop_preserves_final_text() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });
    state.update_final("hello.");
    let view = state.stop();
    assert_eq!(view.final_text.as_deref(), Some("hello."));
}

#[test]
fn final_event_settles_then_listens() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });
    state.update(|view| view.status = LiveSessionStatus::Speaking);
    let view = state.update_final("hello.");
    assert_eq!(view.status, LiveSessionStatus::Settling);
    let view = state.return_to_listening();
    assert_eq!(view.status, LiveSessionStatus::Listening);
    assert_eq!(view.final_text.as_deref(), Some("hello."));
}

#[test]
fn stream_crash_blocks_without_losing_final_text() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });
    state.update_final("kept.");
    let view = state.block_with_error("Live stream stopped.");
    assert_eq!(view.status, LiveSessionStatus::Blocked);
    assert_eq!(view.final_text.as_deref(), Some("kept."));
}

#[test]
fn stream_crash_retires_runtime_handles() {
    let mut inner = LiveRuntimeInner::for_test();
    inner.has_capture_for_test = true;
    inner.has_stream_for_test = true;
    inner.mark_stream_crashed_for_test();
    assert!(!inner.has_capture_for_test);
    assert!(!inner.has_stream_for_test);
}

#[test]
fn level_updates_can_mark_speaking() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });
    state.update(|view| view.status = LiveSessionStatus::Listening);
    let view = state.update_level(0.35);
    assert_eq!(view.status, LiveSessionStatus::Speaking);
    assert_eq!(view.level, Some(0.35));
}
```

- [ ] **Step 2: Run tests and see failure**

```powershell
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml live::runtime
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml stop_preserves_final_text
```

Expected: compile/test failure before implementation.

- [ ] **Step 3: Implement `LiveRuntime` shell**

Add:

```rust
pub struct LiveRuntime {
    inner: Mutex<LiveRuntimeInner>,
}

struct LiveRuntimeInner {
    session: u64,
    capture: Option<cpal::Stream>,
    stream: Option<LiveStreamProcess>,
    cancelled: Arc<AtomicBool>,
    writer: Option<std::thread::JoinHandle<()>>,
    reader: Option<std::thread::JoinHandle<()>>,
    level: Option<std::thread::JoinHandle<()>>,
    vad_segments: Vec<VadSegment>,
    last_used: std::time::Instant,
}
```

Methods:

- `new()`
- `is_active()`
- `start_local(app: tauri::AppHandle, selected_device_id: Option<String>) -> Result<(), String>`
- `stop()`
- `unload_if_idle(threshold: Duration)`
- `shutdown()`
- `handle_stream_crash(app: tauri::AppHandle, session: u64, message: &str)`

Add a tiny `VadSegment { start_ms: u64, end_ms: u64 }` seam and keep it empty until Rust Silero lands. Do not emit fake segments.

- [ ] **Step 4: Implement audio helpers**

Implement `downmix_to_mono`, `f32_to_i16_le_bytes`, `rms_level`, and simple 16 kHz linear resampling in `runtime.rs`. Keep helpers pure and unit-tested.

- [ ] **Step 5: Implement capture bridge**

Use `cpal` to open the resolved input device at start time. The callback does a bounded handoff only; an audio worker performs mono/resample/PCM conversion and sends session-tagged PCM to the writer. A writer thread writes bytes to the session child stdin. A level thread throttles level snapshots, calls `LiveSessionState::update_level`, and emits `live-session` events. A reader thread reads stdout lines, calls `parse_stream_event`, and updates `LiveSessionState` via `app.state::<LiveSessionState>()`.

Use a session token before emitting snapshots.

On stdout EOF, read error, stdin write failure, or child exit, the runtime must:

1. Verify the session token is still current.
2. Drop the active CPAL stream so the mic is cold.
3. Stop/retire writer, reader, and level workers for that session.
4. Retire the crashed `LiveStreamProcess` so the next start creates a fresh session child.
5. Call `LiveSessionState::block_with_error("Live stream stopped.")`.
6. Emit the blocked snapshot without clearing `final_text`.

- [ ] **Step 6: Update state helpers**

Add helpers on `LiveSessionState`:

- `clear_for_new_session`
- `update_level`
- `update_partial`
- `update_final`
- `return_to_listening`
- `block_with_error`

Change `stop()` so it clears mic-hot fields and `partial_text`, but keeps `final_text`. `update_level` should set `Speaking` when the level is above the chosen speech threshold and `Listening` when it falls back below the idle threshold. `update_final` should set `Settling`; the reader thread should call `return_to_listening` after emitting the settling snapshot.

- [ ] **Step 7: Run focused tests**

```powershell
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml live
```

Expected: live runtime/state tests pass.

- [ ] **Step 8: Commit**

```powershell
git add desktop/src-tauri/src/live/mod.rs desktop/src-tauri/src/live/runtime.rs desktop/src-tauri/src/live/state.rs
git commit -m "feat: add local live audio runtime"
```

---

### Task 4: Wire Start/Stop Commands And Overlay Text

**Files:**
- Modify `desktop/src-tauri/src/lib.rs`
- Modify `desktop/src/components/live/live-overlay.tsx`

- [ ] **Step 1: Write/update tests**

Add Rust tests that verify runtime error mapping still distinguishes busy/server/setup errors and that `start_live_intent` blocks when setup is missing. Add a React-side test only if an existing test file already covers live overlay text; otherwise keep this as build-verified UI.

- [ ] **Step 2: Wire Tauri state**

Manage `LiveRuntime` in `run()`:

```rust
.manage(live::runtime::LiveRuntime::new())
```

Create shared Rust helpers used by both Tauri commands and the global shortcut handler:

```rust
fn start_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    runtime: &live::runtime::LiveRuntime,
    stt: &stt::dispatch::SttState,
    orchestrator: &runtime::RuntimeOrchestratorState,
) -> live::state::LiveSessionView

fn stop_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    runtime: &live::runtime::LiveRuntime,
    orchestrator: &runtime::RuntimeOrchestratorState,
) -> live::state::LiveSessionView
```

Update `start_live_session` to call the shared start helper. It should:

1. Reject if `stt_state.is_transcribing()`.
2. Resolve setup state.
3. Block if fallback is not ready.
4. Mark runtime fallback/live active.
5. Clear previous text for a new session.
6. Call `LiveRuntime::start_local`.
7. Emit the resulting live snapshot.

Update `stop_live_session` to call the shared stop helper. Update the global shortcut handler so push-to-talk/toggle uses the same helpers instead of directly calling `start_live_intent(&live)` or `live.stop()`.

- [ ] **Step 3: Add idle/app-exit cleanup**

Extend the existing sidecar monitor or add a small monitor thread to call `LiveRuntime::unload_if_idle(Duration::from_secs(600))`. On app exit, call `LiveRuntime::shutdown()`.

- [ ] **Step 4: Improve overlay text projection**

Show accumulated final text and the current partial separately. Keep the collapsed tier terse. Expanded tier should not prefer partial text so strongly that final text disappears.

- [ ] **Step 5: Run checks**

```powershell
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml live
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml runtime_error_mapping_keeps_server_and_binary_errors_distinct
pnpm -C desktop test
pnpm -C desktop build
```

Expected: Rust live tests, frontend tests, and frontend build pass.

- [ ] **Step 6: Commit**

```powershell
git add desktop/src-tauri/src/lib.rs desktop/src/components/live/live-overlay.tsx
git commit -m "feat: wire live Moonshine fallback"
```

---

### Task 5: Verify And Smoke Test

**Files:**
- Modify docs only if smoke test discovers a known limitation worth documenting.

- [ ] **Step 1: Run full checks**

```powershell
pnpm -C desktop test
pnpm -C desktop build
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml
cargo clippy --locked --manifest-path desktop\src-tauri\Cargo.toml --all-targets -- -D warnings
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 2: Manual live smoke**

Run the app locally, start live from the overlay/settings path, speak one short English sentence, and verify:

- status reaches `listening`/`speaking`
- partial or final text appears
- stop makes the mic cold and retires the session CrispASR stream child
- final text remains visible after stop
- no batch/file transcription starts

- [ ] **Step 3: Record latency note**

Add a short note to the PR/body or final report:

```text
Live smoke: <CPU/GPU path>, first text in <rough seconds>, latency acceptable: yes/no, stop made mic cold: yes/no, session stream retired safely: yes/no.
```

## Spec Coverage Review

| Spec area | Covered by plan |
|-----------|-----------------|
| Phase 3a docs amendment | Task 1 |
| Session-bound local Moonshine stream child | Tasks 2-4 |
| Selected/default mic capture | Task 3 |
| PCM conversion/resampling | Task 3 |
| Partial/final JSONL parsing | Task 2 |
| Live view snapshots | Tasks 3-4 |
| Busy/setup/server route guardrails | Task 4 |
| Punctuation stays enabled | Task 2 |
| Stop keeps final text and cools mic | Tasks 3-5 |
| No local Cohere/server fake | Constraints and Task 4 |
| Verification/smoke | Task 5 |

## Self-Review

- No task adds server WSS, Opus, diarization, Scribe, save-audio, or text injection.
- The plan adds two Rust files because the current `state.rs` is only a snapshot and the existing HTTP sidecar cannot handle stdio streaming.
- Tests are focused on command construction, parser behavior, audio conversion, and state transitions.
- The plan originally targeted warm stream reuse, but implementation review found no safe CrispASR reset/ack boundary for delayed stdout. The shipped Phase 3a path retires the stream child on stop/start boundaries and leaves warm reuse as a follow-on optimization.
