# Task 4 Report: Extract a Preallocated CPAL Capture Adapter

## Status

DONE

## RED

Inherited controller RED command:

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::capture
```

Observed failures before correction:

- `live/runtime.rs` retained a duplicated, obsolete device lookup helper that called `default_input_device` and `input_devices` without `cpal::traits::HostTrait` in scope.
- Stale runtime tests referenced the removed `LiveRuntimeInner.audio` field twice.
- A stale raw-slot test called removed `claim_raw_audio_slot` three times.

The inherited adapter also failed strict clippy for a manual multiple-of check and a nine-argument capture worker helper.

## GREEN

- Removed the obsolete runtime device/sample helpers so capture lookup remains exclusively behind `live::devices::resolve_capture_device`.
- Replaced the stale audio-worker test with a `CapturePorts` lifecycle test that verifies returned-buffer ownership and clean worker exit after packet disconnect.
- Kept capture callback work to fixed-buffer conversion/copy, position tracking, loss recording, and non-blocking channel operations.
- Replaced the manual modulo check with `is_multiple_of` and grouped worker inputs in a private `CaptureWorkerContext` to satisfy strict clippy without changing public APIs.

## Requirement Audit

- Exactly eight buffers are preallocated; a capture test checks eight distinct fixed-capacity allocations.
- Buffer capacity uses `fixed_frames * channels`, with an `8192` fallback for default CPAL buffer sizing.
- Oversized callbacks are discarded and report exact frame loss without growing a buffer.
- Pool exhaustion reports exact frame loss, and subsequent packets prove source positions advance through loss.
- Full and disconnected packet sends reclaim the packet buffer and report `SinkUnavailable` loss.
- Downmixing, resampling, normalization, level calculation, PCM storage, and ASR queueing stay in `run_capture_worker`, outside the callback.
- `CaptureAdapter::shutdown` drops the stream before joining the worker; the runtime lifecycle test verifies the packet worker returns its buffer and joins after disconnect.
- `LiveRuntime::start_local`, stop behavior, and product-visible live state transitions remain unchanged.

## Verification

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::capture
# PASS: 9 passed, 0 failed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime
# PASS: 16 passed, 0 failed

cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml
cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml -- --check
git diff --check
# PASS

cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
# PASS

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
# PASS: 317 unit tests and 1 parity test passed
```

## Files

- `desktop/src-tauri/src/audio/capture.rs`
- `desktop/src-tauri/src/audio/mod.rs`
- `desktop/src-tauri/src/live/devices.rs`
- `desktop/src-tauri/src/live/runtime.rs`
- `docs/archive/implementation-evidence/task-4-capture-adapter-report.md`

## Self-Review

- The old runtime lookup was removed instead of importing `HostTrait` back into runtime, leaving one device-resolution boundary.
- The callback has no `collect`, blocking send, or capacity growth path; it reuses preallocated `Vec<f32>` allocations.
- Worker-side processing retains existing downmix/resample/normalization, level, disk-buffer, and stream-send behavior.
- Changes stay within Task 4; no Task 5 coordinator work was introduced.

## Concerns

- Hardware microphone behavior was not exercised in this environment. Synthetic callback boundary and runtime lifecycle coverage, strict clippy, and the full locked suite pass.

---

## Review Repair: Loss Draining and Worker Hardening

### RED

- Added regression coverage before the repair for malformed callback frame accounting, capture-worker join panic reporting, registration-counter exhaustion recovery, sustained loss beyond the 64-run and 256-ticket capacities, no-packet timeout draining, periodic sustained loss with source-position checks, and a synthetic packet-worker panic.
- The initial RED run failed because the loss callback/timeout loop and explicit capture-worker join result did not exist; the synthetic callback harness was not available to runtime tests.
- The first runtime GREEN attempt exposed a real test-model error: a rendezvous packet channel can hand a packet directly to a worker waiting in `recv_timeout`, so it did not create the expected loss. The sustained test now deliberately holds one packet worker while producing bounded loss batches and verifies every delivered snapshot.

### GREEN

- `run_capture_packet_loop` drains `CapturePorts.losses` before each receive and once more on exit, including idle timeouts and packet disconnect. Exact snapshots are logged as capture degradation; invalid timing uses `spawn_stream_crash_handler` and exits the packet worker.
- `LossAccumulator::record` no longer panics or leaves a drainer waiting after ticket/counter exhaustion. It marks sticky invalid state, returns `InvalidTiming` on drain, and resets its fixed atomic handoff state for subsequent records.
- Malformed callback lengths account for `ceil(samples / channels)` discontinuity frames, advance the source position, and allow later callbacks to continue. Callback arithmetic overflow marks explicit invalid timing for runtime handling.
- Capture and runtime worker joins return explicit errors. Capture drops the stream before joining, includes a join failure when `stream.play` fails, and runtime stop logs shutdown failures without changing the public stop result.
- Packet-worker panics are caught and sent through the stream crash handler before the worker exits. Poisoned live PCM state is an explicit crash path rather than a worker `expect` panic.

### Verification

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::capture
# PASS: 11 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::timeline
# PASS: 28 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime
# PASS: 19 passed

cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml
cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml -- --check
git diff --check
# PASS

cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
# PASS

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
# PASS: 322 unit tests and 1 parity test passed
```

### Self-Review

- Callback work remains fixed-buffer conversion/copy, source accounting, atomic loss recording, and non-blocking channel operations; it has no allocation, blocking send, or callback panic path.
- Loss snapshots are drained at every loop turn and final exit. The fixed-capacity accumulator reports invalid timing instead of retaining unbounded entries, panicking, or stranding a drain.
- Capture stream teardown drops the CPAL stream before joining. Worker panic and join errors are handled explicitly; synthetic tests cover both paths without microphone hardware.

---

## Review Closure: Resumable Loss Drain

### RED / GREEN

- Added deterministic barrier coverage for a pre-target registration that must return `Pending`, a held post-flip entrant that cannot delay the fixed old handoff, and repeated polls that cannot flip a second generation.
- Replaced the coordinator spin waits with `LossAccumulator::try_drain`, which keeps one bounded pending handoff under coordinator-only mutex state. Each poll either returns `Pending` immediately, reports `Empty`, or returns the exact completed snapshot/error.
- Preserved `drain()` for compatibility as a blocking wrapper over `try_drain`; the live packet worker now exclusively polls `try_drain`, so packet disconnect and capture shutdown can complete while a callback publication remains pending.
- Added a packet-loop disconnect regression that holds a pre-target callback at a barrier and proves worker exit without sleeping or waiting for that callback.

### Verification

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::timeline
# PASS: 26 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::capture
# PASS: 11 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime
# PASS: 20 passed

cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml -- --check
git diff --check
# PASS

cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
# PASS

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
# PASS: 321 unit tests and 1 parity test passed
```

### Concerns

- Hardware microphone behavior remains unexercised. The existing worker-failure test remains synthetic; this hard-checkpoint repair did not add a new hardware-free production `LiveRuntime` retirement seam.

---

## Final Repair: Monotonic Registration Tickets

### RED

- `audio::timeline` initially failed 2 of 29 tests: the implementation reset registration counters after `u64` exhaustion, and clearing ticket cells let the held callback/new-generation handoff lose its required monotonic ticket state.
- The deterministic barrier regression holds the final valid registration at capacity while an overflowing callback marks the handoff invalid. It proves repeated `try_drain` calls remain `Pending` without a second generation flip, then the valid target completes, the handoff returns `InvalidTiming`, and the held callback plus the next record are emitted exactly once in the next generation.

### GREEN

- Removed `registration_resetting` and `reset_registration_counters`; `registration_started`, `registration_drained`, and completion tickets are never reset or cleared.
- Capacity exhaustion does not reserve or overwrite a ticket. It stays fail-visible until the fixed valid target drains, advances `registration_drained`, returns `InvalidTiming`, and permits modulo reuse only after that floor advancement.
- True registration-counter exhaustion is terminal: callbacks return without panicking and future drains explicitly return `InvalidTiming`.
- Restored next-generation held-callback coverage; added deterministic reset-window, coordinator-contention, and poisoned-coordinator regressions.

### Final Totals

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::timeline
# PASS: 29 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::capture
# PASS: 11 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime
# PASS: 20 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml old_reset_window
# PASS: 1 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml concurrent_try_drain_returns_pending_while_the_coordinator_is_contended
# PASS: 1 passed

cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml -- --check
git diff --check
cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
# PASS

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
# PASS: 324 unit tests and 1 parity test passed
```

### Self-Review

- `try_drain` retains its non-blocking polling behavior. It holds one fixed target until registrations and writers complete; it does not reset state, wait in callbacks, or clear live ticket cells.
- A contended coordinator returns `Pending`; a poisoned coordinator returns `InvalidTiming` without panicking.
- Scope is limited to `audio/timeline.rs` and this Task 4 report. No Task 5 work was added.

### Concerns

- Hardware microphone behavior remains unexercised; focused capture and runtime lifecycle tests remain synthetic.

---

## Final Narrow Repair: Terminal Loss Generation Exhaustion

### RED

- Added direct `LossAccumulator` coverage with `active_generation` set to `u64::MAX`.
- The named `generation_exhaustion` test failed before the repair: the first `try_drain()` returned `InvalidTiming` but changed the active generation from `u64::MAX` to `0`.

### GREEN

- Added a sticky `generation_exhausted` state. An attempted handoff from `u64::MAX` now returns `InvalidTiming` without changing `active_generation`.
- `record()` checks that terminal state before ticket registration, generation reads, writer registration, or slot publication. Later records are fixed-operation no-ops and cannot use a wrapped/reused slot.
- `try_drain()` checks terminal exhaustion before and after its non-blocking coordinator lock; `drain()` therefore returns `InvalidTiming` on the exhausting call and every later call without another flip or terminal-state clear.
- Added a near-limit positive control: `u64::MAX - 1` advances once to `u64::MAX`, drains the old slot normally, then becomes terminal on the following advance attempt.
- Existing pending handoff and monotonic-ticket tests remain green.

### Verification

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml generation_exhaustion
# RED before repair: failed because active_generation wrapped to 0

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml generation_
# PASS: 3 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::timeline
# PASS: 31 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::capture
# PASS: 11 passed

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime
# PASS: 20 passed

cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml
cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml -- --check
git diff --check
# PASS

cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
# PASS

cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
# PASS: 326 unit tests and 1 parity test passed
```

### Self-Review

- The terminal flag is write-once for the accumulator lifetime and no code path clears it.
- The overflow path never stores a replacement generation, so `active_generation` stays at `u64::MAX` and neither slot can be reused through wraparound.
- The change is confined to `audio/timeline.rs` and this Task 4 report; focused capture/runtime behavior remains covered without product-scope changes.

### Concerns

- Hardware microphone behavior remains unexercised; capture and runtime lifecycle coverage is synthetic. No unresolved generation-exhaustion concern remains in the requested scope.
