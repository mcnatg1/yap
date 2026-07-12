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

---

## Durability Follow-up

### Implementation

- Successful commits now remove only their owned private append journal after publication. Partial finalization retains it, and a valid commit continues to suppress a residual journal from partial recovery.
- Manual and retention deletion intents now publish from a unique synced private staging file through no-replace publication. Resume re-proves the current commit hash, manifest session, required names/hashes, and retention metadata before deleting; commit absence permits only intent cleanup after all listed physical entries are absent.
- Corrupt final intents are quarantined only when all original evidence remains hash-valid. Truncated/corrupt intents after deletion progress remain on disk, fail closed, and are surfaced via the bounded saved-session catalog maintenance warnings.
- Canonical transcript reads and previews consume validated no-follow handles. Canonical owned audio bypasses the external playback registry; path-only open/reveal/asset flows revalidate immediately before dispatch. Same-user mutation after that final pathname check remains a documented residual because that actor can directly alter the file.

### Focused Evidence

- `audio::recording`: successful journal retirement, partial retention, and committed-residue suppression.
- `live::recordings`: corrupt-intent quarantine before deletion, truncated-final retention after progress, catalog warning persistence, and the existing replacement/crash resume coverage.

### Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` - pass (420 library tests plus 1 parity integration test, default parallel execution).
- `cargo fmt --all --check --manifest-path .\desktop\src-tauri\Cargo.toml` - pass.
- `cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings` - pass.
- `pnpm --dir desktop test` - pass (101 tests).
- `pnpm --dir desktop build` - pass.
- `git diff --check` - pass.

### Residual

- Same-user filesystem access is not a security boundary. The implementation rejects malformed, cross-session, path-replacement, reparse, and stale intent inputs, but cannot prevent that same user from directly deleting or replacing their own recording after pathname validation for OS-owned dispatch APIs.

---

## Damaged-State And Lifecycle Repair (2026-07-11)

### Status

DONE

### Implementation

- `RecordingScan` now separates hash-valid complete captures, current-writer partials, hash-valid recovered-partial commits, and damaged complete commits. Damaged commits carry a bounded validation reason and cannot become recoverable/TTL cleanup targets; the catalog retains every artifact and surfaces a bounded maintenance warning.
- Recovered-partial commits are validated against their exact WAV, partial sidecar, hashes, size, session identity, and timestamp. They remain recoverable/deleteable outside the ordinary 24-hour private-partial cleanup path.
- Private deletion leftovers use strict staging/quarantine grammars, a 128-entry scan budget, age and current-process checks, and the existing no-follow/identity-aware remove primitive. Unknown, malformed, active, too-new, and nonregular entries are retained with bounded warnings.
- Corrupt-final intent replacement now writes and syncs staging before quarantine. The quarantine retains a verified identity/hash receipt; successful publication removes that exact object, while publication failure restores it when the destination is free or retains evidence otherwise.
- Removed `RECEIPT_HANDLE_COUNT` and `ReceiptHandleProbe`. Direct behavior tests move/replace sidecar and transcript paths after receipt creation and prove revalidation fails closed.

### Regression Coverage

- Corrupt complete-commit JSON, audio hash mismatch, sidecar corruption, residual journal, expired private residue, damaged catalog warning visibility, and recovery/delete preservation for a valid old recovered-partial commit.
- Old foreign private staging/quarantine collection, active-process and malformed-file preservation, corrupt-intent repeated retry cleanup, corrupt-final success cleanup, and existing pre/post publication replacement failures.

### Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::recording` passed three default-parallel runs: 44 tests each.
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings` passed three default-parallel runs: 56 tests each.
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` passed: 426 library tests and 1 parity integration test.
- `cargo fmt --all --check --manifest-path .\desktop\src-tauri\Cargo.toml` passed.
- `cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings` passed.

### Residual

- The same-user filesystem mutation boundary remains unchanged. The repair retains uncertain evidence rather than deleting it, and external mutation after final path-only OS dispatch validation remains outside this local authorization boundary.

---

## Final Bounded Cleanup-Lifecycle Fix (2026-07-11)

### Implementation

- Reconciliation now recognizes the generic private delete quarantine emitted by `recording::quarantine_open_regular_artifact`: `.<exact-yap-artifact>.delete-<pid>-<nonce>`. The strict grammar accepts only known session-bound Yap artifact basenames and keeps nested, malformed, active, too-new, nonregular, reparse, and unknown entries as evidence.
- Candidate selection occurs only after strict foreign, old, regular filtering. A fixed-size ordered set keeps memory bounded, scans beyond unrelated directory entries, and makes later catalog passes deterministic progress for overflow batches.
- Corrupt-intent replacement first reconciles strict prior intent quarantines. A missing final restores the newest verified evidence; an extant final retires verified superseded evidence before another replacement can create new evidence.
- Maintenance-warning assembly reserves the cap's leading slots for damaged committed sessions, followed by pending deletion and stale-cleanup warnings. No frontend contract changed.

### Focused Coverage

- Generic stale quarantines for audio, sidecar, transcript, immutable transcript revision, commit, journal, and intent are collected; nested, malformed, active, recent, and nonregular entries remain untouched.
- A directory with 256 unrelated entries before 129 eligible leftovers cleans 128 in the first pass and the remainder in the next pass.
- A missing final intent restores the newest verified quarantine, and three post-publication corrupt-intent retries retain exactly one evidence quarantine.
- A cap-full warning set still emits damaged committed-session evidence first.

### Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::recording` - pass three times, 44 tests per default-parallel run.
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings` - pass three times, 61 tests per default-parallel run.
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` - pass, 431 library tests plus 1 parity integration test.
- `cargo fmt --all --check --manifest-path .\desktop\src-tauri\Cargo.toml` - pass.
- `cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings` - pass.
- `git diff --check` - pass.

---

## Final Deletion And Authorization Repair

### Status

DONE

### Implementation

- Replaced the frontend pathname-based `delete_history_entry_files` route with `delete_saved_live_session(session_id)`, registered through the main-window command boundary.
- Added a schema-v1 deletion intent for manual and expired-meeting cleanup. It validates the commit, exact transcript revision chain, and bounded same-session artifacts before publishing; it removes artifacts hash-safely, then the commit, then the intent.
- Reconciliation resumes pending intents before committed-recording scanning. Failed cleanup preserves the intent and surfaces a session warning when the commit is still valid; malformed/orphaned intents are logged and not trusted.
- Updated owned-path file actions so canonical committed audio/transcript validation is required inside the Yap recordings directory. Timestamp-era and other uncommitted artifacts cannot fall through into the playback registry; registered external imports still do.

### Focused Coverage

- Manual deletion removes bound audio, sidecar, transcript, immutable revision, safely attributable polished derivative, commit, and intent.
- A crash-style state after audio removal is resumed during listing; mismatched replacements are preserved with the intent retained; forged intents cannot name an arbitrary or cross-session file.
- Audio-only committed deletion, expired-meeting shared cleanup, dictation no-retention survival, timestamp-path action rejection, and canonical audio/transcript resolution are covered.

### Final Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` - pass (414 library tests plus 1 parity integration test, default parallel execution).
- `cargo fmt --all --check --manifest-path .\desktop\src-tauri\Cargo.toml` - pass.
- `cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings` - pass.
- `pnpm --dir desktop test` - pass (101 tests).
- `pnpm --dir desktop build` - pass.

---

## Deletion Cleanup Concurrency And Fairness Repair (2026-07-11)

### Implementation

- Serialized deletion-intent publication and reconciliation behind one process-wide ownership lock. A catalog scan can reclaim same-process evidence only while no same-process writer can still be publishing it; fresh evidence from another process remains untouched until the existing age safeguard permits reconciliation.
- Replaced lexically fixed cleanup batches with a bounded per-directory rotating cursor. Each pass still retains at most 128 candidates in each bounded partition, but advances the cursor even when selected artifacts cannot be removed so a permanently failing early candidate cannot starve later cleanup work.
- Kept cursor state process-local and bounded to 64 directories. Restarting resets the maintenance cursor, which is acceptable because cleanup state is advisory; durable recording and deletion truth remains in commit manifests and deletion intents.
- Preserved the prerelease canonical-format decision: timestamp-era `live-<timestamp>` files are neither migrated nor indexed by the runtime, and no compatibility adapter was reintroduced.

### Regression Coverage

- A publication barrier test holds an active intent quarantine while a competing writer starts and proves reconciliation cannot reclaim the in-flight evidence.
- Fresh foreign-process intent evidence remains retained, while abandoned same-process evidence is reconciled under exclusive ownership during catalog maintenance.
- A full failed cleanup batch advances on the next pass and selects a candidate beyond the original lexical budget.

### Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings` passed three default-parallel runs: 65 tests each.
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` passed: 435 library tests and 1 parity integration test.
- `cargo fmt --all --check --manifest-path .\desktop\src-tauri\Cargo.toml` passed.
- `cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings` passed.
- `git diff --check` passed.

---

## Verified Task 7 Review Fixes (2026-07-11)

### Implementation

- Pending deletion intents now use the same bounded, per-directory rotating selection as private deletion leftovers. The shared cursor state retains at most two names for each of at most 64 directories, so permanently failing early intents cannot starve later valid intents.
- Manual deletion holds process-wide deletion ownership from intent publication through resume completion. Reconciliation and standalone resume calls use the same ownership boundary; explicit `while_owned` helpers prevent recursive locking.
- Fresh foreign evidence TTL handling, hash-bound deletion checks, and the non-migrated/unindexed canonical pre-release timestamp files remain unchanged.

### Regression Coverage

- 128 persistently failing intents followed by a valid intent resume the valid deletion on the next catalog reconciliation pass.
- A resume waits for an existing deletion owner, and a catalog reconciliation started after a manual intent is published cannot complete until that manual deletion finishes.

### Verification

- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings::tests` - pass, 68 tests.
- `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml` - pass, 438 library tests plus 1 parity integration test.
- `cargo fmt --all --check --manifest-path .\desktop\src-tauri\Cargo.toml` - pass.
- `cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings` - pass.
