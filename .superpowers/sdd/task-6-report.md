# Task 6 Implementation Report

## RED Evidence

Added `audio::recording` tests before the production recording API existed. The initial focused test run failed with unresolved `StreamingRecording`, `CaptureStatus`, `CommitFaultPoint`, and `scan_recordings` symbols, proving the new contract was not covered by existing code.

## Implementation Summary

- Added `audio::recording` with streamed PCM16 WAV writing, one-second `sync_data` cadence, 32-bit WAV data-length refusal, streamed SHA-256 hashing, compact recovery journal snapshots, commit-last capture sidecar publication, and manifest/hash/schema validation.
- Added test-only faults for append, periodic flush, WAV patch, audio sync, sidecar sync, finalized-artifact rename, commit sync, and commit rename. Each produces an explicit partial candidate and never a complete history item.
- Replaced the ten-minute `RecordedPcmBuffer` with `RecordingSinkHandle`. It owns the bounded recording sink, drains on its worker, and supports concurrent/idempotent finalization.
- Kept capture/ASR stopping in `LiveRuntime::stop`; `save_session_files` performs recording finalization after `run_completion_effects_with` has injected the transcript.
- Updated history scanning to treat valid new-style `commit.json` as the sole completion authority while retaining legacy `live-<timestamp>[-suffix].wav/.txt` scanning.
- Kept `.txt` publication atomic and added create-new `transcript.rN.json` revisions containing Task 2 validated lineage plus text filename/hash, local Nemotron identity, revision timestamp, and capture sidecar hash.

## Commands And Results

| Command | Result |
| --- | --- |
| `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::recording` | Passed: 7 tests |
| `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings` | Passed: 15 tests |
| `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` | Passed: 347 unit tests and 1 integration test |
| `cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml -- --check` | Passed |
| `cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings` | Passed |
| `git diff --check` | Passed |

## Artifact And Durability Ownership Review

- The bounded recording receiver is consumed only by the recording worker; the CPAL callback and coordinator do no disk I/O.
- The worker creates `live-<session>.wav.part` and `live-<session>.capture.journal.part`, appends audio and compact metadata, then finalizes `.wav`, `.capture.json`, and lastly `.commit.json`.
- A valid same-directory commit manifest, matching artifact names, lengths, hashes, and sidecar schema is required before a new-style recording appears in history.
- Interrupted artifacts remain recovery candidates. A partial audio finalization leaves transcript history with a visible audio-save warning rather than treating the transcript or WAV as committed audio.
- Transcript injection remains ahead of recording finalization. A transcript revision failure leaves the committed capture intact.
- Windows parent-directory syncing is explicitly recorded as unsupported by the standard-library path, leaving the documented residual power-loss window after rename.

## Scope Review

Changed only `desktop/src-tauri/src/audio/recording.rs`, `audio/mod.rs`, `live/recordings.rs`, and `live/runtime.rs`, plus this required report. No server connector, frontend, diarization, model runtime, or unrelated documentation was changed.

## Concerns

Windows cannot reliably `sync_all` a parent directory with the portable standard library. File data and metadata are synced before rename, but a sudden power loss still has the documented directory-entry durability window.
