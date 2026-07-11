# Task 7 Implementation Report

## Status

DONE

## RED Evidence

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings::tests::committed_history_exposes_its_hash_validated_commit_path` initially failed because `captureCommitPath` serialized as `null`.
- `pnpm --dir desktop test -- history.test.ts history-utils.test.ts` initially failed because recovery metadata was discarded and a recoverable row projected as `complete`.
- The new saving-claim regression first failed while it had no final transcript, then passed after the test supplied the completed transcript; this confirmed injection is deliberately conditional on final text.

## Implementation Summary

- Added `LiveStopResult` and a per-session stop-completion lease. Direct/racing `LiveRuntime::stop()` callers share one cached stream/finalization result; action completion drains the stream, injects, then finalizes and saves exactly once through the saving claim.
- Added committed-history commit paths, separate recoverable-session discovery, native recover/delete commands, 24-hour partial expiry reconciliation, and no-follow/identity-checked partial WAV repair and deletion helpers.
- Added optional `captureCommitPath` and `recoveryState` history fields. Existing localStorage rows remain valid. Recoverable rows are compact `Partial` rows in the existing History table with Recover/Delete menu commands.

## Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::actions` - pass
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings` - pass
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime` - pass
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` - pass, 400 tests
- `cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings` - pass
- `pnpm --dir desktop test` - pass, 97 tests
- `pnpm --dir desktop build` - pass
- `cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml` - pass
- `git diff --check` - pass

## Ownership And Security Review

- Command registration stays main-window authorized in `lib.rs`.
- Recovery/delete validate opaque session IDs, rediscover the session through Task 6 scanning, resolve only expected basenames under the configured recordings directory, use no-follow regular-artifact checks, and avoid deleting unrelated files.
- Recovery writes only minimal partial sidecar metadata and does not synthesize journal gaps or timeline information. Complete history still comes exclusively from hash-valid complete commits.
- Repeated stop finalization is cached, and action-level side effects are guarded by the existing saving claim.

## UI Compatibility Review

- LocalStorage remains backward-readable, but strict timestamp-named pre-release WAV/TXT rows are excluded regardless of default, custom, or relative location; unrelated imports remain available.
- No new card, modal, dependency, or breakpoint was introduced. The history list uses the existing menu primitives and Phosphor icons.

## Scope Review

- Changes are confined to the requested live/history surfaces plus narrowly necessary Task 6 artifact helpers in `audio/recording.rs` for no-follow update/delete/hash operations.
- No server connector, model runtime, diarization, SQLite, upload queue, or unrelated UI was changed.

## Concerns

- The current live capture/commit data model does not carry meeting-retention expiry metadata. Partial artifact expiry is implemented, but hash-bound expiry deletion for committed meeting sessions and the corresponding dictation-survival regression are not implemented and must be completed before this should be treated as a fully compliant Task 7 handoff.

---

## Review-Fix Evidence

### Status

DONE

### Retention And Artifact Security

- Complete capture sidecars and manifests now carry the same optional, immutable `SessionMetadata`; a new capture always binds the persistent recording `SessionId`, dictation mode/live origin, and the active PushToTalk or Toggle trigger before publication. Older manifests without metadata continue to validate and are never retention-deleted.
- Reconciliation deletes only hash-validated, expired `Meeting` plus `LiveCapture` commits with a parseable expiry at or before the reconciliation clock. Future meetings, imported origins, dictation, absent metadata, and malformed expiry values remain outside the deletion predicate.
- Cleanup verifies manifest bytes before it starts, uses manifest-bound audio/sidecar hashes, validates the complete highest transcript chain before removing transcript artifacts, and deletes the commit last. Failure remains visible as an `Expired meeting cleanup is pending` warning.
- Recovery validates the opened WAV as canonical RIFF/WAVE PCM mono 16 kHz/16-bit audio with aligned physical lengths before patching. It accepts only matching placeholder/final length pairs, does not mutate invalid input, recognizes an uncommitted final WAV as recoverable, and supports retry/delete without inventing gaps.
- Artifact deletion quarantines an owned path, compares the quarantined handle identity to the original handle, restores/fails closed on a replacement, and only removes the proven object. Tests cover the replacement barrier.

### Transcript And UI Review

- History determines the highest numbered transcript revision first and requires every revision through that exact highest candidate, predecessor hash link, capture-sidecar hash, and transcript text hash to validate. A corrupt or missing high revision leaves audio visible without silently trusting a lower revision.
- Native refreshes replace only Rust-managed commit/recoverable rows after the new native list succeeds. Legacy/imported entries and hidden tombstones remain intact. Recoverable rows cannot select, preview, open, copy, or play; their compact menu exposes only Recover and Delete with existing Phosphor icons.

### Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::actions` - pass (7)
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings` - pass (37 focused)
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime` - pass (27)
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` - pass (407)
- `cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings` - pass
- `cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml -- --check` - pass
- `pnpm --dir desktop test` - pass (99)
- `pnpm --dir desktop build` - pass
- `git diff --check` - pass

### Scope And Concern Review

- Preserved Task 6 commit-last/no-follow/hash validation, Task 7 saving lease/exactly-once stop effects, command authorization, 24-hour partial cleanup, transcript independence, and the restrained history surface.
- No server, model, diarization, or SQLite changes. No remaining Task 7 concerns identified.

---

## Repair Evidence: Canonical Recording Format

### Decision

- Yap is pre-release: the only normal recording format is a hash-valid current commit manifest plus its bound artifacts. Timestamp-named WAV/TXT files are not history, recovery, or retention-cleanup candidates.
- No runtime migration adapter exists. Timestamp-named WAV/TXT files remain unindexed and physically untouched: no product path renames, deletes, adopts, warns about, or recovers them. A future explicit operator tool is out of scope.

### Repair Coverage

- A public final WAV becomes recoverable only with current-writer partial lineage; a timestamp-named final WAV is ignored and cannot enter 24-hour cleanup.
- Timestamp-named files are also rejected by the normal history-file deletion command, so the pre-release break does not leave a second mutation path behind.
- Recovered partials now use the single `recoverable` UI state. Older persisted `recovered` rows are also gated as partial, so neither state can preview, copy, open, reveal, hide, play, or use normal deletion.
- Expired-meeting deletion now enumerates transcript artifacts first and deletes nothing unless their exact contiguous revision set and hash chain validate. A failure stays visible as `Expired meeting cleanup is pending`.
- Runtime visible history and reconciliation discard strict timestamp-named WAV/TXT rows by name and expected basenames, independent of their directory. Normal Yap actions require a canonical commit or a partial-recovery state.

### Focused Regression Evidence

- `audio::recording`: timestamp-named final WAVs are ignored; a current-writer final WAV with its journal but no partial sidecar remains recoverable.
- `live::recordings`: WAV-only and WAV/TXT pre-release artifacts remain untouched and unindexed; retry after recovered-commit publication returns the verified saved partial; incomplete expired-meeting transcript chains retain all artifacts.
- `history` and `history-utils`: pre-release localStorage rows are hidden from runtime history and all partial states are action-gated.

### Final Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::recording` - pass (40 focused tests).
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings` - pass (42 focused tests).
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml file_actions::tests` - pass (40 focused tests).
- `pnpm --dir desktop test -- history.test.ts history-utils.test.ts` - pass (100 tests across the configured unit suite).
- `cargo fmt --all --check --manifest-path .\desktop\src-tauri\Cargo.toml` - pass.
- `cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings` - pass.
- `pnpm --dir desktop build` - pass.
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` - pass (414 library tests plus 1 parity integration test, default parallel execution).
- `pnpm --dir desktop test` - pass (100 tests).
