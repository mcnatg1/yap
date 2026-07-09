# Task 6 Report: Add Windowing And Tail Rules For Future Server Chunks

## Status

DONE

## Scope Completed

- Added `ChunkWindowConfig` and `build_manifest_windows(...)` in `desktop/src-tauri/src/audio/manifest.rs`.
- Kept the helper pure and in-memory only. No upload, route, contract, runtime, or server behavior was added.
- Returned `Vec::new()` for empty frame input.
- Used stable session-relative frame windows by sorting on `(start_ms, sequence, duration_ms)` before chunking.
- Marked mixed-session or mixed-sample-rate input as `VadKind::Error` chunks instead of silently merging heterogeneous frame runs.
- Treated missing, uncovered, and explicit error VAD ranges as target-window error chunks.
- Closed chunks on speech boundaries before `max_window_ms` when VAD guidance ended earlier.
- Preserved final speech tail audio by extending only into available trailing silence frames, capped by `ChunkWindowConfig.tail_padding_ms` and `max_window_ms`.
- Kept `VadDecision.end_ms` as the classifier-provided boundary and only extended chunk coverage into additional available frames. This avoids mutating VAD segment endpoints and prevents reapplying tail padding to the same covered range.
- Preserved silence marker manifests only when `preserve_silence_markers` is `true`.
- Added a small `AudioFrame::end_ms()` helper in `desktop/src-tauri/src/audio/frame.rs` so the manifest helper could clamp to actual frame coverage with saturating arithmetic.

## Files Changed

- `desktop/src-tauri/src/audio/manifest.rs`
- `desktop/src-tauri/src/audio/frame.rs`

## Tests Added

- `build_manifest_windows_returns_empty_for_empty_frames`
- `build_manifest_windows_uses_target_windows_for_vad_error_fallback`
- `build_manifest_windows_preserves_specific_error_vad_metadata`
- `build_manifest_windows_closes_on_vad_boundaries_before_max_window`
- `build_manifest_windows_adds_final_word_tail_padding_from_available_frames`
- `build_manifest_windows_preserves_silence_markers_only_when_requested`
- `build_manifest_windows_marks_mixed_sample_rates_as_error_chunks`
- `end_ms_uses_saturating_frame_coverage`

## Verification

Ran:

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml manifest_windows
```

Result:

- Passed: 7
- Failed: 0
- Notes: compile completed with existing dead-code warnings only; no new runtime wiring was introduced.

## Commit

- `053e3f3` `Add pure manifest chunk windowing helper`

## Self-Review

- Scope stayed within the allowed audio files after reverting an out-of-scope formatter reorder in `desktop/src-tauri/src/lib.rs`.
- The helper remains deterministic and side-effect free.
- Error chunks now preserve specific `VadKind::Error` RMS values when VAD supplied them, and fall back to `0.0` only when no specific decision applies.
- Tail extension is clamped to actual frame coverage and chunk max duration, with the VAD segment end left unchanged.
- I did not touch live runtime, upload plumbing, saved-session contracts, or server APIs.

## Concerns

- The current implementation uses per-frame overlap classification with `Speech > Silence > Error` tie-breaking and conservative whole-input erroring for mixed session/sample-rate inputs. That matches the task constraints and tests, but if a later phase needs finer-grained overlap arbitration or mixed-run salvage, that behavior should be revisited alongside the future server contract.

## Critical Review Fix Addendum

Addressed reviewer findings on top of `053e3f3`:

- Prevented manifest tail padding from being applied twice when VAD speech windows already cover padded tail audio. The manifest now resolves the earliest overlapping non-speech boundary, accounts for already-covered tail time, and only extends into additional available frames when padding is still owed.
- Changed speech chunk windowing so `target_window_ms` is no longer a hard cap for speech. Speech now grows up to `max_window_ms` and still closes early when a VAD boundary lands sooner.
- Strengthened regression coverage with tests for already-padded VAD speech, speech growth to `max_window_ms`, and a truly saturating `AudioFrame::end_ms()` case.

Verification command run:

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::
```

Output summary:

- Passed: 30
- Failed: 0
- Included coverage for `audio::manifest::tests::build_manifest_windows_*` and `audio::frame::tests::end_ms_uses_saturating_frame_coverage`
- Notes: completed with existing dead-code warnings in the audio modules; no server/upload/live-runtime behavior was added
