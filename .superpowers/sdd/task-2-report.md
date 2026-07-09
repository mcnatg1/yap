# Task 2 Report: Define Frame And Manifest Data Shapes

## Summary

Implemented the Task 2 transport-neutral audio metadata and manifest shapes for the desktop audio scaffold without adding runtime routing or server connector behavior.

## Files Changed

- `desktop/src-tauri/src/audio/frame.rs`
- `desktop/src-tauri/src/audio/manifest.rs`
- `desktop/src-tauri/src/audio/vad.rs`

## What Changed

### `desktop/src-tauri/src/audio/frame.rs`

- Added metadata-only `AudioFrame` with:
  - `session_id: u64`
  - `sequence: u64`
  - `sample_rate_hz: u32`
  - `channels: u16`
  - `start_ms: u64`
  - `duration_ms: u32`
  - `sample_count: usize`
- Added `AudioCodec` limited to `PcmS16Le`.
- Added local-only `AudioPurpose` with:
  - `LocalFallback`
  - `CaptureEnvelope`
- Added `VadSegment`.
- Added `RetryMetadata`.
- Added `AudioChunkEnvelope`.
- Implemented `AudioFrame::duration_ms_from_samples(sample_count, sample_rate_hz)`.
- Implemented `AudioChunkEnvelope::from_frames(...) -> Option<Self>`.

### `desktop/src-tauri/src/audio/manifest.rs`

- Added `AudioSessionEnvelope`.
- Added `AudioSource` with:
  - `Live`
  - `Recording`

### `desktop/src-tauri/src/audio/vad.rs`

- Added minimal placeholder `VadKind` compatibility enum:
  - `Speech`
  - `Silence`

This was the smallest compatible addition needed so manifest-facing `VadSegment` could compile and serialize cleanly without pulling runtime VAD behavior into this task.

## Requirement Checks

- Used session-relative monotonic `start_ms` fields only; no wall-clock time was introduced in these types.
- Used `u64` for both `session_id` and `sequence`.
- Kept `AudioFrame` metadata-only with no retained sample vectors.
- Did not add server connector code, ASR routing, server route enums, or package churn.
- Kept `AudioPurpose` local capture metadata only.
- Did not define any server route variants such as `serverLive` or `serverBatch`.
- Generated `chunk_id` exactly as:
  - `"{session_id}-{sequence_start}-{duration_ms}"`
- Generated `RetryMetadata.idempotency_key` exactly as:
  - `"{session_id}-{sequence_start}-{chunk_id}"`
- Kept `AudioCodec` limited to `PcmS16Le`.

## Test Coverage Added

- Duration math:
  - sample count to milliseconds
  - zero sample rate guard
- Empty frame handling:
  - `from_frames` returns `None`
- Retry metadata:
  - deterministic `chunk_id`
  - deterministic `idempotency_key`
- Serialization names:
  - chunk envelope camelCase fields
  - codec snake_case value
  - purpose camelCase value
  - VAD kind snake_case value
  - session envelope camelCase fields
  - audio source snake_case value

## Verification Run

Command:

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::
```

Result:

- Passed
- `9` tests passed
- `0` failed

## Self-Review

- Scope stayed within the task-owned files plus the allowed minimal `vad.rs` compatibility shim.
- No runtime capture or transport behavior was added.
- The manifest and frame shapes remain transport-neutral and local-first.
- The only compiler feedback during test execution was expected `dead_code` warnings because these new types are defined ahead of runtime integration in later tasks.

## Concerns

- New audio metadata types currently produce `dead_code` warnings because Task 2 defines the shapes before downstream runtime wiring exists. This does not block the task and is expected at this phase boundary.
