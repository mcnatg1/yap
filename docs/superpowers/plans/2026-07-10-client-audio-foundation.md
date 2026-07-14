# Client Audio Foundation Implementation Plan

> **Historical record — current authority (2026-07-14):** This plan preserves
> the landed audio-foundation recipe. References below to focused-field or
> synthesized text injection are superseded. Current behavior uses
> native-confirmed bounded shortcut enrollment and clipboard-only delivery; use
> [ADR 0013](../../adr/0013-global-hotkey-injection.md) as authority.

> **Implementation status (2026-07-12):** Capture/session contracts, exact loss accounting, bounded sink fan-out, crash-safe streaming recording, immutable commit metadata, and recovery/deletion are implemented and unit/integration tested. Speaker inference, system loopback, server transport, and the durable job ledger remain separate future gates; unchecked boxes below are not reliable landed-state evidence.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish Yap's source-aware desktop capture foundation so dictation and meetings share one bounded, testable audio path with explicit gaps and crash-safe recordings.

**Architecture:** Keep CPAL as a thin capture adapter. Move session identity, timeline, preprocessing, sink fan-out, and recording persistence into focused Rust audio modules. The callback writes only into a preallocated buffer pool and an atomic loss accumulator. A coordinator creates prepared frames and fans them into independently bounded recording, local-ASR, future-evidence, and future-transport sinks. Recording bytes stream to disk and become complete only after an immutable capture sidecar and commit manifest are atomically published.

**Tech Stack:** Tauri 2, Rust 2021 standard library, existing `cpal`, existing `sha2`, existing `serde`/`serde_json`, direct `time 0.3.53` using the version already present in the lockfile, sherpa-onnx local streaming ASR, React 19, TypeScript, Vitest, Playwright, WebdriverIO.

## Global Constraints

- Apply ADR 0013 for client-owned hotkeys and safe delivery, ADR 0014 for the client/server boundary, ADR 0019 for the local streaming model, and ADR 0020 for capture authority and persistence.
- Treat ADR 0002 runtime details and ADR 0015's earlier diarization split as historical. They do not authorize a second local ASR path or persistent local voice profiles.
- Preserve current push-to-talk, hands-free, overlay, safe clipboard-delivery, model-download, playback, and history behavior.
- Do not add a speaker model, embedding runtime, system-loopback capture, network connector, SQLite, Opus encoder, or inference framework.
- Do not run official imported-file transcription through local Nemotron.
- Do not serialize sample buffers, embeddings, credentials, raw OS device labels, or mutable retry state into the immutable capture manifest.
- The audio callback must not wait on inference, disk, React, or an ordinary bounded queue to report loss.
- Recording, local ASR, future evidence, and future transport have separate bounded queues and separate degradation results.
- Recording files remain inspectable files. No audio or transcript body belongs in a database.
- Existing single-microphone settings remain readable. Pre-release `live-<timestamp>[-suffix].wav/.txt` artifacts are a deliberate compatibility break: product runtime leaves them physically untouched and never indexes, recovers, retains, renames, deletes, or adopts them. A future explicit operator tool is separate work.
- Phase boundary: this plan completes the client-audio parts of canonical Phase 1 and the capture prerequisites pulled forward from Phases 3 and 5. Phase 8 still begins with anonymous speaker inference.

## Governing Documents

- [ADR 0013: Global hotkey and safe cross-app delivery](../../adr/0013-global-hotkey-injection.md)
- [ADR 0014: Server-tier compute topology](../../adr/0014-server-tier-compute-topology.md)
- [ADR 0019: Local streaming model selection](../../adr/0019-local-streaming-model-selection.md)
- [ADR 0020: Meeting capture and diarization authority](../../adr/0020-meeting-capture-diarization-authority.md)
- [Client recording state machine](../../specs/client-state-machine.md)
- [Local audio preprocessing stack](../../specs/local-audio-preprocessing-stack.md)
- [Source-aware diarization design](../specs/2026-07-10-source-aware-diarization-design.md)

---

## Current Baseline

| Area | Current implementation | Change in this plan |
|------|------------------------|---------------------|
| Capture | `desktop/src-tauri/src/live/runtime.rs` owns CPAL, resampling, levels, recording, and ASR delivery. | Extract CPAL and coordination without changing product gestures. |
| Recording | `RecordedPcmBuffer` retains at most ten minutes of PCM in memory. | Stream PCM to a private partial WAV and finalize with sidecar plus commit manifest. |
| Audio identity | `AudioFrame` has session/sequence but no track identity. `AudioSource::{Live, Recording}` conflates origin with workflow. | Add orthogonal session mode, session origin, trigger mode, track source, and track identity. |
| Backpressure | Raw and ASR channels are bounded, but dropped callback audio is not a first-class timeline event. | Add a callback-safe loss accumulator and deterministic gap events. |
| Manifests | Deterministic VAD window builders exist. Hash, replay key, track, gap, and artifact identity are incomplete. | Extend builders and fail closed on mixed identity or conflicting replay. |
| Recovery | Atomic WAV/TXT final writes exist, but no capture commit protocol exists. | Publish audio, capture sidecar, and commit manifest in a tested order; scan leftovers as partial. |

## Target Module Shape

```text
desktop/src-tauri/src/audio/
  mod.rs
  session.rs       session, origin, trigger, track, and source contracts
  frame.rs         prepared frame metadata and chunk/replay identity
  timeline.rs      monotonic timeline events and atomic callback-loss reporting
  capture.rs       preallocated CPAL adapter boundary
  coordinator.rs   preprocessing and independently bounded sink fan-out
  evidence.rs      anonymous evidence and attribution contracts; no model runtime
  results.rs       immutable transcript and speaker result revisions
  recording.rs     streaming WAV, sidecar, commit, recovery, and fault injection
  preprocess.rs    existing pure conversion/resampling/level helpers
  vad.rs           existing deterministic VAD decisions
  manifest.rs      strict session/chunk/capture manifest builders
desktop/src-tauri/src/install_identity.rs  non-secret local owner namespace
```

`live/runtime.rs` remains the live-session lifecycle and local-ASR owner. `live/recordings.rs` remains the Tauri/history facade. Neither file owns raw capture mechanics after this plan.

## Spec Traceability

| Accepted requirement | Plan coverage |
|----------------------|---------------|
| Mode, origin, trigger, and physical source stay orthogonal | Task 1 |
| Track IDs, content hashes, replay keys, evidence, revisions, and validation are deterministic | Tasks 1-2 |
| Callback loss remains observable when the ordinary queue is full | Tasks 3-4 |
| Recording, ASR, evidence, and transport fail independently | Task 5 |
| More than ten minutes records with bounded memory | Tasks 6 and 8 |
| Completion requires a commit manifest; crashes remain partial | Tasks 6-7 |
| Dictation remains compatible; pre-release artifacts stay outside product runtime | Tasks 1, 7, and 8 |
| No local speaker model or new inference framework lands | Global constraints and final review gate |

---

## Task 1: Add Orthogonal Session And Track Contracts

**Files:**
- Modify: `desktop/src-tauri/Cargo.toml`
- Modify: `desktop/src-tauri/Cargo.lock`
- Create: `desktop/src-tauri/src/install_identity.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Create: `desktop/src-tauri/src/audio/session.rs`
- Modify: `desktop/src-tauri/src/audio/frame.rs`
- Modify: `desktop/src-tauri/src/audio/manifest.rs`
- Modify: `desktop/src-tauri/src/audio/mod.rs`
- Test: `desktop/src-tauri/src/audio/session.rs`
- Test: `desktop/src-tauri/src/audio/frame.rs`
- Test: `desktop/src-tauri/src/audio/manifest.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeSessionToken(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct OwnerNamespace(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TrackId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode { Dictation, Meeting }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOrigin { LiveCapture, ImportedFile }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode { PushToTalk, Toggle }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource { Microphone, SystemLoopback }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrackSource {
    Captured { source: CaptureSource },
    Imported { provenance: ImportedTrackProvenance },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    pub session_id: SessionId,
    pub mode: SessionMode,
    pub origin: SessionOrigin,
    pub trigger_mode: TriggerMode,
    pub started_at_utc: String,
    pub utc_offset_minutes_at_start: Option<i16>,
    pub locale_hint_bcp47: Option<String>,
    pub country_code_hint: Option<String>,
    pub preferred_languages_bcp47: Vec<String>,
    pub app_version: String,
    pub platform: String,
    pub privacy_policy_version: String,
    pub retention_expires_at_utc: Option<String>,
}
```

- [ ] **Step 1: Write failing contract tests**

Add tests for:

```rust
#[test]
fn track_id_rejects_empty_control_or_separator_values();

#[test]
fn session_mode_origin_trigger_and_source_round_trip_independently();

#[test]
fn generated_session_ids_remain_distinct_across_process_and_counter_inputs();

#[test]
fn install_identity_is_stable_across_reopen_and_never_silently_rotates();

#[test]
fn imported_origin_does_not_claim_a_physical_capture_source();

#[test]
fn legacy_live_audio_source_deserializes_as_live_capture();

#[test]
fn metadata_formats_utc_as_rfc3339_and_keeps_timing_monotonic_elsewhere();

#[test]
fn metadata_bounds_language_hints_and_validates_country_without_location_inference();

#[test]
fn meeting_metadata_requires_an_explicit_retention_expiry();

#[test]
fn manifest_device_reference_is_opaque_and_does_not_contain_the_os_label();
```

Run:

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::session
```

Expected before implementation: compilation fails because `audio::session` does not exist.

- [ ] **Step 2: Declare the existing locked time crate directly**

Run from `desktop/src-tauri`:

```powershell
cargo add time@0.3.53 --no-default-features --features formatting,parsing
```

This declares a crate already present transitively in `Cargo.lock`. Use `time::OffsetDateTime` with `time::format_description::well_known::Rfc3339`; do not hand-roll calendar conversion.

- [ ] **Step 3: Implement validated domain types, metadata, and legacy reads**

Implement `SessionId::new`, `SessionId::generate`, `OwnerNamespace::local`, `TrackId::new`, `TrackId::as_str`, `Display`, and `TryFrom<String>`. Session IDs are opaque strings generated as `s-<unix-nanoseconds-hex>-<process-id-hex>-<atomic-counter-hex>` with checked time conversion. Recording allocation uses create-new semantics and advances the counter if any artifact with that session prefix already exists, making collision a handled retry rather than overwrite. Track IDs accept 1-64 ASCII alphanumeric, `_`, and `-` characters. Add `ImportedTrackProvenance::{Unknown, Mixed, UserDeclared(CaptureSource)}` and `CaptureTrackDescriptor` with opaque `device_id`.

`install_identity.rs` loads or creates one non-secret `install-id` under `paths::app_data_dir()` using create-new, flush, and atomic validation. The value uses the same bounded opaque-ID grammar and produces `OwnerNamespace("local:<install-id>")`. A malformed existing file fails visibly and is never silently rotated because that would change replay ownership for existing artifacts. The file contains no username, machine name, hardware ID, contact, credential, or biometric value.

Keep the current `index:label` selector ID readable in local settings for backward compatibility, but never place it in a capture sidecar or transport contract. Derive `CaptureTrackDescriptor.device_id` as `dev-<first-32-hex-of-sha256(install-id || 0x00 || selector-id)>`. The UI may still display the live label locally; manifests and debug snapshots expose only the opaque reference.

Keep `RuntimeSessionToken` as the in-process atomic cancellation/crash token used by `live/runtime.rs`. It is never persisted or sent to the server. `SessionId` is the durable identity used by artifacts, manifests, replay keys, SQLite, and future server calls.

Build `SessionMetadata` through a validating constructor. Accept at most eight preferred-language hints and 35 ASCII characters per BCP 47 hint; allow alphanumeric subtags separated by single hyphens and reject empty/consecutive subtags. Country is omitted by default and, when explicitly configured, must be exactly two ASCII letters normalized to uppercase. Never derive it from IP, device location, timezone, or locale. Use `env!("CARGO_PKG_VERSION")` for app version and `std::env::consts::OS` for platform.

Until a reviewed notice is configured, write `privacy_policy_version: "unconfigured"`; this value is honest metadata and never grants upload, identity, or retention authority. A meeting-mode constructor defaults to a finite RFC 3339 expiry 30 days after start and accepts an explicit reviewed user/organization policy override; it never silently creates perpetual retention. Current dictation may leave retention unset under its existing local-file lifecycle.

Keep serialized values snake_case. Add a private compatibility enum in `manifest.rs` so old `source: "live"` and `source: "recording"` artifacts can be read and projected to `SessionOrigin::{LiveCapture, ImportedFile}`. New writes must use `session_mode`, `session_origin`, and `tracks`.

- [ ] **Step 4: Make frame metadata track-aware**

Change `AudioFrame` to carry `SessionId` and `TrackId`. Add `sequence_end` to `AudioChunkEnvelope`. Keep sample data out of serializable metadata by introducing:

```rust
#[derive(Debug, Clone)]
pub struct PreparedFrame {
    pub metadata: AudioFrame,
    pub samples: std::sync::Arc<[f32]>,
}
```

Update frame and manifest tests so chunk ordering is `(track_id, sequence_start, start_ms)`, not only sequence number.

- [ ] **Step 5: Run focused and full Rust tests**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::session
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml install_identity
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::frame
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::manifest
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
```

Expected: all tests pass, and existing single-track fixtures still deserialize.

- [ ] **Step 6: Commit**

```powershell
git add desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock desktop/src-tauri/src/install_identity.rs desktop/src-tauri/src/lib.rs desktop/src-tauri/src/audio/session.rs desktop/src-tauri/src/audio/frame.rs desktop/src-tauri/src/audio/manifest.rs desktop/src-tauri/src/audio/mod.rs
git commit -m "Define source-aware audio session contracts"
```

---

## Task 2: Make Chunk, Evidence, And Result Contracts Fail Closed

**Files:**
- Create: `desktop/src-tauri/src/audio/evidence.rs`
- Create: `desktop/src-tauri/src/audio/results.rs`
- Modify: `desktop/src-tauri/src/audio/mod.rs`
- Modify: `desktop/src-tauri/src/audio/frame.rs`
- Modify: `desktop/src-tauri/src/audio/manifest.rs`
- Test: `desktop/src-tauri/src/audio/evidence.rs`
- Test: `desktop/src-tauri/src/audio/results.rs`
- Test: `desktop/src-tauri/src/audio/frame.rs`
- Test: `desktop/src-tauri/src/audio/manifest.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkReplayKey {
    pub schema_version: u16,
    pub owner_namespace: OwnerNamespace,
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub sequence_start: u64,
    pub sequence_end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentIdentity {
    pub sha256: String,
    pub byte_length: u64,
}

pub enum ReplayDecision { Idempotent, Distinct }
pub enum ReplayConflict { SameKeyDifferentContent }

pub enum ResultAuthority { LocalProvisional, LocalReconciled, ServerAuthoritative, UserCorrected }
pub enum ResultStatus { Complete, Partial }

pub enum SpeakerAttribution {
    Unknown,
    SessionSpeaker { session_speaker_id: String },
    Named(NamedSpeakerAssertion),
}

pub struct SpeakerEvidence {
    pub track_id: TrackId,
    pub start_ms: u64,
    pub end_ms: u64,
    pub local_slot_id: Option<String>,
    pub model: ModelRevision,
    pub quality: EvidenceQuality,
    pub confidence: Option<f32>,
}

pub struct SpeakerResultRevision {
    pub session_id: SessionId,
    pub revision: u64,
    pub authority: ResultAuthority,
    pub capture_sidecar_sha256: String,
    pub previous_result_sha256: Option<String>,
    pub status: ResultStatus,
    pub speaker_turns: Vec<SpeakerTurn>,
    pub aligned_words: Vec<AlignedWord>,
    pub model_provenance: Vec<ModelRevision>,
}
```

- [ ] **Step 1: Add the replay-matrix tests first**

Add exact cases:

```rust
#[test]
fn same_key_and_hash_is_idempotent();

#[test]
fn same_key_and_different_hash_is_a_conflict();

#[test]
fn different_keys_with_the_same_hash_remain_distinct();

#[test]
fn builder_rejects_cross_session_cross_track_and_sequence_regression();

#[test]
fn builder_rejects_impossible_or_overlapping_frame_timing();

#[test]
fn client_evidence_builder_can_emit_only_unknown_or_session_speaker();

#[test]
fn result_revisions_require_capture_hash_and_monotonic_revision();

#[test]
fn evidence_and_result_json_contains_no_embedding_or_exemplar_values();
```

Run:

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml replay_
```

Expected before implementation: the new tests fail to compile.

- [ ] **Step 2: Replace string-derived chunk identity with typed identity**

Make chunk construction receive a `ChunkBuildContext` containing owner namespace, session mode/origin, track descriptor, route, artifact ID, and encoded audio bytes. Compute SHA-256 with the existing `sha2` crate. Build `chunk_id` from the replay key only; keep the byte hash separate.

```rust
pub struct ChunkBuildContext<'a> {
    pub owner_namespace: &'a OwnerNamespace,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub track: &'a CaptureTrackDescriptor,
    pub route: AudioRoute,
    pub audio_artifact_id: &'a str,
    pub encoded_audio: &'a [u8],
}
```

Return `Result<AudioChunkEnvelope, ManifestError>` instead of silently relabeling or returning `None` for invalid non-empty input.

- [ ] **Step 3: Add data-only evidence and result contracts**

Put `ModelRevision`, `EvidenceQuality`, `SpeakerEvidence`, `SpeakerAttribution`, `SpeakerTurn`, `AlignedWord`, and `NamedSpeakerAssertion` in `evidence.rs`. Put `ResultAuthority`, `ResultStatus`, `TranscriptResultRevision`, and `SpeakerResultRevision` in `results.rs`.

Intervals are end-exclusive and reject `end_ms <= start_ms`. A client constructor can create only `Unknown` or `SessionSpeaker`; `Named` requires a server-result constructor carrying identity ID, profile/model/calibration revision, confidence, purpose-grant ID, and revocation epoch. These modules contain no embedding vector type, model loader, clustering state, or persistence port. Debug and serialized forms expose provenance and counts only.

- [ ] **Step 4: Validate timing and continuity without hiding gaps**

Require monotonically increasing sequences and end-exclusive timing. Permit a sequence or time discontinuity only when a matching `AudioGap` covers it. Reject mixed sample rates unless the session contains a prior conversion/configuration revision for that track.

- [ ] **Step 5: Run tests and serialization snapshots**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::frame
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::manifest
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::evidence
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::results
cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
```

Expected: replay and validation tests pass; clippy reports no warnings.

- [ ] **Step 6: Commit**

```powershell
git add desktop/src-tauri/src/audio/evidence.rs desktop/src-tauri/src/audio/results.rs desktop/src-tauri/src/audio/mod.rs desktop/src-tauri/src/audio/frame.rs desktop/src-tauri/src/audio/manifest.rs
git commit -m "Harden audio manifest identity"
```

---

## Task 3: Add A Monotonic Timeline And Callback-Safe Loss Accumulator

**Files:**
- Create: `desktop/src-tauri/src/audio/timeline.rs`
- Modify: `desktop/src-tauri/src/audio/mod.rs`
- Test: `desktop/src-tauri/src/audio/timeline.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapCause { CallbackPoolExhausted, OversizedCallback, DeviceDiscontinuity, SinkUnavailable }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioGap {
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub source_position_frames: u64,
    pub dropped_frames: u64,
    pub cause: GapCause,
    pub generation: u64,
}

pub enum TimelineEvent {
    TrackConfigured(TrackConfigurationRevision),
    ClockMapped(ClockMappingRevision),
    Frame(AudioFrame),
    Gap(AudioGap),
}
```

- [ ] **Step 1: Write deterministic timeline tests**

Cover monotonic frame conversion, end-exclusive intervals, per-track sequence ownership, contiguous same-cause gap coalescing, and rejection of non-contiguous coalescing.

- [ ] **Step 2: Write race-oriented accumulator tests**

The accumulator uses two preallocated atomic slots. The callback records into the active slot without allocation. The coordinator flips the active slot, waits only on the coordinator side for prior writers to exit, and drains the inactive slot. Add tests proving:

```rust
#[test]
fn saturated_handoff_reports_the_exact_dropped_interval();

#[test]
fn callback_updates_racing_a_drain_survive_in_the_next_generation();

#[test]
fn draining_an_empty_accumulator_returns_none();
```

Use a barrier-controlled test thread rather than timing sleeps.

- [ ] **Step 3: Implement `SessionClock`, `Timeline`, and `LossAccumulator`**

Use source frame positions for callback accounting and convert to milliseconds only in the coordinator. Saturating arithmetic must produce a visible invalid-timing error instead of wrapping.

```rust
pub struct LossSnapshot {
    pub first_source_position_frames: u64,
    pub dropped_frames: u64,
    pub cause: GapCause,
    pub generation: u64,
}

impl LossAccumulator {
    pub fn record(&self, source_position_frames: u64, dropped_frames: u64, cause: GapCause);
    pub fn drain(&self) -> Option<LossSnapshot>;
}
```

- [ ] **Step 4: Run focused stress tests**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::timeline
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml callback_updates_racing -- --nocapture
```

Run the race test 100 times from PowerShell:

```powershell
1..100 | ForEach-Object {
  cargo test --quiet --locked --manifest-path .\desktop\src-tauri\Cargo.toml callback_updates_racing
  if ($LASTEXITCODE -ne 0) { throw "race test failed on iteration $_" }
}
```

Expected: every run passes without a lost or duplicated gap generation.

- [ ] **Step 5: Commit**

```powershell
git add desktop/src-tauri/src/audio/timeline.rs desktop/src-tauri/src/audio/mod.rs
git commit -m "Track explicit audio timeline gaps"
```

---

## Task 4: Extract A Preallocated CPAL Capture Adapter

**Files:**
- Create: `desktop/src-tauri/src/audio/capture.rs`
- Modify: `desktop/src-tauri/src/audio/mod.rs`
- Modify: `desktop/src-tauri/src/live/devices.rs`
- Modify: `desktop/src-tauri/src/live/runtime.rs`
- Test: `desktop/src-tauri/src/audio/capture.rs`
- Test: `desktop/src-tauri/src/live/runtime.rs`

**Interfaces:**

```rust
pub struct CapturePacket {
    pub source_position_frames: u64,
    pub channels: u16,
    pub sample_rate_hz: u32,
    pub samples: Vec<f32>,
}

pub struct CaptureAdapter {
    stream: cpal::Stream,
    worker: std::thread::JoinHandle<()>,
}

pub struct CapturePorts {
    pub packets: std::sync::mpsc::Receiver<CapturePacket>,
    pub returned_buffers: std::sync::mpsc::SyncSender<Vec<f32>>,
    pub losses: std::sync::Arc<LossAccumulator>,
}
```

- [ ] **Step 1: Test the buffer-pool boundary without a microphone**

Add a synthetic callback harness proving the pool allocates all buffers during construction, reuses returned buffers, reports exact loss when empty, and reports oversized callbacks rather than growing a buffer inside the callback.

- [ ] **Step 2: Move CPAL device/config setup into `audio/capture.rs`**

Move `open_capture`, sample-format stream construction, raw callback handling, and source-position tracking out of `live/runtime.rs`. Keep device selection in `live/devices.rs`; expose one resolved `cpal::Device` plus config boundary rather than duplicating lookup.

Preallocate eight buffers. Derive capacity from the fixed device buffer size when available; otherwise use 8192 samples per callback. If a callback exceeds capacity, record `OversizedCallback` and discard that callback explicitly.

- [ ] **Step 3: Keep conversion and inference off the callback**

The callback may copy samples into an available preallocated buffer and call `try_send`. Downmix, resample, normalization, level calculation, `Arc<[f32]>` creation, disk writes, and ASR sends happen in the coordinator worker.

- [ ] **Step 4: Integrate behind the existing `LiveRuntime::start_local` API**

Keep the public start/stop API stable in this task. `LiveRuntime` may temporarily consume `CapturePacket` directly until Task 5 adds the coordinator. Remove the old `RawAudio` type and duplicated callback state only after the adapter tests pass.

- [ ] **Step 5: Run capture and live regressions**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::capture
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime
cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
```

Expected: tests pass and no product-visible live state changes.

- [ ] **Step 6: Commit**

```powershell
git add desktop/src-tauri/src/audio/capture.rs desktop/src-tauri/src/audio/mod.rs desktop/src-tauri/src/live/devices.rs desktop/src-tauri/src/live/runtime.rs
git commit -m "Extract the CPAL capture adapter"
```

---

## Task 5: Add Independent Bounded Audio Sinks

**Files:**
- Create: `desktop/src-tauri/src/audio/coordinator.rs`
- Modify: `desktop/src-tauri/src/audio/mod.rs`
- Modify: `desktop/src-tauri/src/live/runtime.rs`
- Modify: `desktop/src-tauri/src/live/stream.rs`
- Test: `desktop/src-tauri/src/audio/coordinator.rs`
- Test: `desktop/src-tauri/src/live/runtime.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SinkKind { Recording, LocalAsr, SpeakerEvidence, ServerTransport }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkOutcome {
    pub kind: SinkKind,
    pub accepted_frames: u64,
    pub dropped_frames: u64,
    pub closed: bool,
    pub error: Option<String>,
}

pub struct CoordinatorPorts {
    pub recording: BoundedSink<PreparedFrame>,
    pub local_asr: Option<BoundedSink<PreparedFrame>>,
    pub speaker_evidence: Option<BoundedSink<PreparedFrame>>,
    pub server_transport: Option<BoundedSink<PreparedFrame>>,
}
```

- [ ] **Step 1: Write fan-out failure tests first**

Add synthetic-frame tests for:

```rust
#[test]
fn recording_continues_when_local_asr_is_absent();

#[test]
fn stalled_asr_does_not_block_recording_or_callback_intake();

#[test]
fn one_sink_failure_does_not_close_other_sinks();

#[test]
fn finalization_closes_every_sink_exactly_once();

#[test]
fn composed_result_marks_only_the_failed_or_degraded_sinks();
```

- [ ] **Step 2: Implement the coordinator worker**

Consume `CapturePacket`, drain loss before each accepted packet, downmix per physical track, resample to 16 kHz for the current local-ASR/recording path, compute level, create one `PreparedFrame`, and fan out `Arc` clones with non-blocking bounded sends. Derive `start_ms` from source position plus the active clock mapping, never from the count of samples that survived backpressure; a gap must not compress the session timeline. Reset resampler state only after emitting new track-configuration and clock-mapping revisions.

The initial queue capacities are explicit constants with tests: recording 128 frames, local ASR 64, evidence 32, transport 64. They are starting values, not architecture promises. Log queue high-water marks in debug builds and expose them to tests.

- [ ] **Step 3: Adapt the local ASR stream as one sink**

Keep Nemotron-specific chunk accumulation in `live/stream.rs`. Replace direct callback-to-`StreamMessage` delivery with a local-ASR sink worker that converts `PreparedFrame.samples` into the recognizer's existing stream messages.

When local ASR backs up, mark `transcription_degraded` once and keep recording. Do not turn an ASR drop into a fake recording gap.

- [ ] **Step 4: Reserve evidence and transport ports without workers**

Represent disabled optional sinks as `None`; do not spawn idle placeholder threads. The coordinator contract must support adding them later without changing capture or recording ownership.

- [ ] **Step 5: Run deterministic and live tests**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::coordinator
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::runtime
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::stream
```

Expected: all sink isolation tests pass; local transcript tests remain green.

- [ ] **Step 6: Commit**

```powershell
git add desktop/src-tauri/src/audio/coordinator.rs desktop/src-tauri/src/audio/mod.rs desktop/src-tauri/src/live/runtime.rs desktop/src-tauri/src/live/stream.rs
git commit -m "Fan out bounded audio sinks"
```

---

## Task 6: Stream Recordings And Publish A Commit Manifest Last

**Files:**
- Create: `desktop/src-tauri/src/audio/recording.rs`
- Modify: `desktop/src-tauri/src/audio/mod.rs`
- Modify: `desktop/src-tauri/src/live/recordings.rs`
- Modify: `desktop/src-tauri/src/live/runtime.rs`
- Test: `desktop/src-tauri/src/audio/recording.rs`
- Test: `desktop/src-tauri/src/live/recordings.rs`

**Artifacts:**

```text
live-<session-id>.wav.part                 private streaming audio
live-<session-id>.capture.journal.part     private recovery journal
live-<session-id>.wav                      finalized audio
live-<session-id>.capture.json             immutable compact capture sidecar
live-<session-id>.commit.json              completion authority, published last
live-<session-id>.txt                      transcript result, preserved for current UX
live-<session-id>.transcript.r1.json        immutable result revision referencing capture
```

**Commit manifest:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus { Complete, Partial }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureCommitManifest {
    pub schema_version: u16,
    pub session_id: SessionId,
    pub status: CaptureStatus,
    pub audio_file: String,
    pub audio_sha256: String,
    pub audio_bytes: u64,
    pub capture_sidecar_file: String,
    pub capture_sidecar_sha256: String,
    pub committed_at_utc: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResultRevision {
    pub schema_version: u16,
    pub session_id: SessionId,
    pub revision: u64,
    pub authority: ResultAuthority,
    pub capture_sidecar_sha256: String,
    pub text_file: String,
    pub text_sha256: String,
    pub status: ResultStatus,
    pub model_id: String,
    pub model_revision: String,
    pub created_at_utc: String,
}

```

`TranscriptResultRevision`, `ResultAuthority`, and `ResultStatus` are the types created in `audio/results.rs` by Task 2; recording persistence consumes them rather than defining a second wire shape.

- [ ] **Step 1: Add fault-injection tests for every persistence boundary**

Define a test-only `CommitFaultPoint` port and assert the scan result after failure at: append, periodic flush, WAV header patch, audio sync, sidecar sync, final artifact rename, commit sync, and commit rename.

Every case must produce either one hash-valid committed session or one explicit partial recovery candidate. No case may appear as a complete legacy session merely because a `.wav` or `.txt` file exists.

- [ ] **Step 2: Implement a streaming PCM16 WAV writer with bounded memory**

Write a 44-byte placeholder header, append PCM16 frames, and call `sync_data` at one-second audio intervals. Finalization patches RIFF/data lengths, calls `sync_all`, computes SHA-256 by streaming the file, and atomically renames it. Reject recordings beyond the WAV 32-bit data-length limit with a visible partial result rather than wrapping lengths.

- [ ] **Step 3: Append a compact recovery journal**

Persist track configuration, clock mapping, gaps, sequence coverage, and sink degradation. Do not write sample arrays or one JSON event per ordinary audio frame. Coalesce contiguous gaps and sequence coverage so metadata remains bounded over a four-hour session.

- [ ] **Step 4: Publish sidecar and commit in the ADR 0020 order**

Commit manifests contain validated same-directory file names, never absolute or parent-relative paths.

1. Close producers and drain the loss accumulator.
2. Flush and finalize audio.
3. Write, flush, hash, and rename the capture sidecar.
4. Write, flush, and atomically rename `commit.json` last.
5. Attempt the strongest available parent-directory sync. Treat unsupported Windows directory sync as a documented residual power-loss window, not a successful durability guarantee.

- [ ] **Step 5: Preserve transcript publication as a separate result**

Keep `.txt` publication atomic. Write it after capture completion and keep injection before potentially slower file finalization, as `live/actions.rs` does now. Then append `transcript.r<revision>.json` with the text hash, capture-sidecar hash, local Nemotron model/revision, authority `local_provisional`, and complete/partial status. Use create-new semantics and monotonically increasing revision numbers; never overwrite an earlier result. A transcript failure must not invalidate a committed recording; an audio commit failure must make the history result partial.

- [ ] **Step 6: Remove the ten-minute in-memory PCM buffer**

Delete `MAX_RECORDED_PCM_SECONDS`, `MAX_RECORDED_PCM_BYTES`, `RecordedPcmBuffer`, `take_recorded_pcm`, and `restore_recorded_pcm` from `live/runtime.rs`. Replace them with one owned `RecordingSinkHandle` whose stop/finalize method is idempotent.

- [ ] **Step 7: Run persistence and full Rust checks**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::recording
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
```

Expected: fault injection passes and no ten-minute-cap warning remains.

- [ ] **Step 8: Commit**

```powershell
git add desktop/src-tauri/src/audio/recording.rs desktop/src-tauri/src/audio/mod.rs desktop/src-tauri/src/live/recordings.rs desktop/src-tauri/src/live/runtime.rs
git commit -m "Commit live recordings crash safely"
```

---

## Task 7: Integrate Finalization With Live State And History

**Files:**
- Modify: `desktop/src-tauri/src/live/actions.rs`
- Modify: `desktop/src-tauri/src/live/recordings.rs`
- Modify: `desktop/src-tauri/src/live/runtime.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src/live.ts`
- Modify: `desktop/src/history.ts`
- Modify: `desktop/src/lib/history-utils.ts`
- Modify: `desktop/src/components/panels/history-panel.tsx`
- Test: `desktop/src-tauri/src/live/actions.rs`
- Test: `desktop/src-tauri/src/live/recordings.rs`
- Test: `desktop/tests/unit/history.test.ts`
- Test: `desktop/tests/unit/history-utils.test.ts`

**Interfaces:**

```rust
pub struct LiveStopResult {
    pub stream: StreamFinishStatus,
    pub recording: RecordingFinalizeResult,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableLiveSession {
    pub session_id: String,
    pub name: String,
    pub audio_partial_path: Option<String>,
    pub journal_partial_path: Option<String>,
    pub reason: String,
    pub expires_at_ms: u64,
}
```

- [ ] **Step 1: Add live completion regression tests**

Cover repeated stop, stream crash during stop, transcript-only completion, committed audio with transcript failure, partial audio with final transcript, injection before file finalization, late ASR events after finalization, expired meeting cleanup, and proof that dictation without retention metadata is not deleted.

- [ ] **Step 2: Make stop/finalize idempotent**

Replace `LiveRuntime::stop() -> StreamFinishStatus` with `LiveStopResult`. A finalization lease may run once per session. Repeated or racing stop calls return the already-computed result and cannot rename, emit, inject, or close a sink twice.

- [ ] **Step 3: Scan committed and partial sessions separately**

`list_saved_live_sessions` discovers sessions only from hash-valid `.commit.json` files, validates the highest transcript result revision when present, and reads creation time from committed metadata. Timestamp-named WAV/TXT pairs are physically untouched and never scanned, adopted, warned about, or treated as history/recovery/retention input. Add `list_recoverable_live_sessions` only for private artifacts from the current writer. Validate every path remains inside the effective Yap recordings directory before returning it.

Delete partial artifacts whose recorded recovery expiry is older than 24 hours during startup/list reconciliation. Cleanup must resolve every candidate under the Yap recordings directory, ignore unknown files, and report failures without hiding still-recoverable sessions.

Apply the same path-checked reconciliation to committed meeting sessions whose manifest retention expiry has passed. Delete only files named and hash-bound by that commit plus its transcript/result artifacts, and fail closed when any transcript artifact exists but the exact highest contiguous hash-valid revision chain cannot be proven. Never delete an external/imported source or a dictation session with no expiry. Frontend history reconciliation removes rows after Rust confirms the artifacts are gone.

- [ ] **Step 4: Add minimal recovery actions**

Add Rust commands:

```rust
recover_live_session(session_id: String) -> Result<SavedLiveSession, String>
delete_recoverable_live_session(session_id: String) -> Result<(), String>
```

Recovery patches a valid partial WAV length, publishes a sidecar and commit marked `partial`, and never invents missing gap metadata. Deletion removes only Rust-resolved files for that session. Surface recoverable sessions in the existing History list with `Partial` status and `Recover` / `Delete` menu actions; do not add a card or modal inside the history surface.

- [ ] **Step 5: Retire pre-release runtime history**

Extend `SavedLiveSession` and `TranscriptHistoryEntry` with optional `captureCommitPath` and `recoveryState`. Strict timestamp-named localStorage WAV/TXT rows are excluded regardless of absolute, custom, or relative path; unrelated imported rows remain available. A recovered partial remains a compact partial row with Recover/Delete-only actions. Existing pre-release `.wav/.txt` pairs without a canonical commit are never history, recovery, retention, or normal deletion candidates.

- [ ] **Step 6: Run Rust and frontend tests**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::actions
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live::recordings
pnpm --dir desktop test -- history.test.ts history-utils.test.ts
pnpm --dir desktop build
```

Expected: all tests and build pass.

- [ ] **Step 7: Commit**

```powershell
git add desktop/src-tauri/src/live/actions.rs desktop/src-tauri/src/live/recordings.rs desktop/src-tauri/src/live/runtime.rs desktop/src-tauri/src/lib.rs desktop/src/live.ts desktop/src/history.ts desktop/src/lib/history-utils.ts desktop/src/components/panels/history-panel.tsx desktop/tests/unit/history.test.ts desktop/tests/unit/history-utils.test.ts
git commit -m "Surface committed and recoverable recordings"
```

---

## Task 8: Prove Boundedness, Compatibility, And Product Behavior

**Files:**
- Create: `desktop/src-tauri/tests/audio_foundation.rs`
- Modify: `desktop/tests/e2e/app.spec.ts`
- Modify: `desktop/tests/wdio/live-overlay.spec.js`
- Modify: `docs/specs/local-audio-preprocessing-stack.md`
- Modify: `docs/superpowers/specs/2026-07-10-source-aware-diarization-design.md`
- Modify: `docs/VOICE-OS-ARCHITECTURE.md`

- [ ] **Step 1: Add a four-hour synthetic boundedness test**

Drive the coordinator with a fake capture source and a counting recording port. Advance source position through four hours without materializing four hours of samples or writing a 460 MB fixture. Assert fixed buffer-pool capacity, fixed queue capacity, bounded coalesced metadata, monotonic timeline, and no retained PCM growth.

- [ ] **Step 2: Add a real short fixture integration test**

Use the existing licensed deterministic WAV fixture. Feed it through the coordinator and recording writer, finalize, reopen the WAV, verify header/data length and SHA-256, parse the sidecar/commit, and assert playback-compatible output.

- [ ] **Step 3: Add desktop behavior assertions**

Playwright: committed rows appear as saved; recoverable rows appear as partial; no nested recovery modal is introduced.

WDIO: start and stop live dictation, assert the overlay returns to idle, assert the main window is not required for capture, and assert a saved event is emitted once.

- [ ] **Step 4: Run the complete verification matrix**

```powershell
pnpm --dir desktop test
pnpm --dir desktop build
pnpm --dir desktop test:e2e
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
cargo clippy --locked --all-targets --manifest-path .\desktop\src-tauri\Cargo.toml -- -D warnings
```

On a Windows machine with the native test build available:

```powershell
pnpm --dir desktop test:desktop:all
```

Expected: all automated checks pass. If native microphone permission prevents WDIO capture, record the exact skipped assertion and complete one manual push-to-talk plus one hands-free recording smoke.

- [ ] **Step 5: Update architecture status only after verification**

Mark the client-audio Phase 1 checklist complete only for track-aware contracts, explicit gaps, independent sinks, crash-safe recording, and removal of the retained-PCM cap. Keep SQLite, server transport, system loopback, anonymous speaker evidence, and model benchmarks unchecked.

- [ ] **Step 6: Commit**

```powershell
git add desktop/src-tauri/tests/audio_foundation.rs desktop/tests/e2e/app.spec.ts desktop/tests/wdio/live-overlay.spec.js docs/specs/local-audio-preprocessing-stack.md docs/superpowers/specs/2026-07-10-source-aware-diarization-design.md docs/VOICE-OS-ARCHITECTURE.md
git commit -m "Verify the client audio foundation"
```

---

## Final Review Gate

- [ ] Every new failure path has a regression test written before its fix.
- [ ] The callback path contains no disk, inference, React, network, blocking lock wait, or fallback allocation.
- [ ] A recording sink failure cannot be presented as complete.
- [ ] An ASR/evidence/transport failure cannot silently truncate the recording.
- [ ] Transcript result revisions reference capture hashes and never rewrite capture history.
- [ ] Same replay key plus different content fails closed.
- [ ] Manifest builders fail closed on cross-session and cross-track contamination.
- [ ] Gaps remain explicit and end-exclusive timing remains monotonic.
- [ ] Dictation behavior and serialized settings remain backward compatible.
- [ ] Pre-release live artifacts remain physically untouched and outside every product runtime path; a future explicit operator tool is out of scope.
- [ ] Capture sidecars contain opaque device references and no raw OS device labels.
- [ ] Dictation injection still uses only final transcript text.
- [ ] No speaker model, SQLite, Opus, server connector, or new inference dependency landed.
- [ ] Docs describe exactly what the verified code now does.

---

## Task 7 Deletion Authorization Repair (2026-07-10)

- Completed history deletion is a Rust-owned `delete_saved_live_session(session_id)` command. The frontend requests deletion only for a native canonical row and sends its opaque session ID; it no longer supplies a transcript pathname to a generic delete route.
- Manual deletion and expired live-meeting retention share one same-directory, schema-versioned deletion intent. The intent binds the session, reason, original commit hash, and a bounded list of exact same-session artifact hashes. Artifacts are removed with no-follow, hash-and-identity-safe quarantine helpers; the commit is always last.
- Startup/list reconciliation resumes valid pending intents before normal commit scanning. Missing artifacts are treated as already removed, while replacements or malformed intents remain on disk with a warning rather than deleting an unverified file.
- Canonical Yap paths resolve through a hash-valid commit. Uncommitted or timestamp-era files inside the recordings directory cannot be read, previewed, polished, opened, revealed, registered for playback, or deleted through product actions. Registered external recordings remain supported.
- Successful commit publication retires only the writer-owned `.capture.journal.part`; partial/failure paths retain it. A valid commit suppresses a crash-residue journal from recovery, and a later deletion intent includes that journal only after parsing its session identity and hashing its exact file.
- Deletion intent publication uses a unique private staging basename, file sync, no-replace publication, and parent sync where supported. A matching valid final intent is idempotent. A corrupt final can be quarantined and replaced only while the original commit and every intent artifact still hash-match; otherwise cleanup fails closed and retains the evidence.
- Every intent resume re-reads, hashes, parses, and binds the current commit before deletion. Retention resumes also re-prove live-meeting expiry authorization. With no commit, the intent is removed only once every listed physical entry is absent; symlinks, reparse points, directories, and inaccessible paths are mismatches rather than absence.
- Read and preview consume the already hash-validated no-follow transcript handle. Polish validates a source handle before its atomic derivative write. Open, reveal, and asset protocol dispatch are pathname APIs, so they revalidate immediately before dispatch; a same-user replacement after that final check remains outside this trust boundary because the same user can directly modify or delete their own files.
- Cleanup failures are returned as bounded `maintenanceWarnings` with the saved-session catalog so the app can show one restrained startup toast even when the corresponding commit is no longer a valid history row.

### Damaged-State And Intent-Lifecycle Follow-up (2026-07-11)

- [x] Model a strict complete commit that fails parsing, schema validation, hash validation, or sidecar validation as a bounded-reason damaged committed session. It is not a partial candidate; its audio and private artifacts remain untouched and the saved-session catalog reports maintenance evidence.
- [x] Recognize hash-valid recovered-partial commits separately from current-writer partials. Recovered commits remain recoverable and deletable even when their residual private artifacts are older than the ordinary partial TTL.
- [x] Bound private deletion staging and quarantine reconciliation to the strict app-owned grammar and fixed scan budget. Only old foreign-process regular artifacts are collected with no-follow and identity-aware removal; active, too-new, malformed, unknown, and reparse entries remain untouched with bounded warnings.
- [x] Stage a replacement deletion intent before quarantining a corrupt final. After publish and verification, remove the exact quarantined artifact with receipt hash and identity checks; failed publication restores verified evidence when possible or retains it for catalog evidence.
- [x] Remove receipt-handle-count instrumentation. Filesystem behavior tests prove receipt-created transcript and sidecar paths can be moved/replaced and that receipt revalidation fails closed afterward.

### Final Bounded Cleanup-Lifecycle Review (2026-07-11)

- [x] Recognize only strict app-owned private deletion forms. Generic delete quarantines use `.<exact-yap-artifact>.delete-<pid>-<nonce>` and recover the exact session-bearing audio, sidecar, transcript, commit, journal, or intent basename; nested, malformed, active, too-new, nonregular, reparse, and unknown entries remain evidence.
- [x] Reconciliation filters to old foreign regular candidates before a deterministic bounded selection. It scans past unrelated files without building an unbounded entry list and continues in later catalog passes when more than one batch is present.
- [x] Before replacing a corrupt intent, reconcile verified strict intent quarantines: restore the newest evidence if the final is missing, or safely retire superseded evidence when the final exists. A failed replacement/retry sequence retains no growing collection of verified intent quarantines.
- [x] Compose catalog maintenance warnings with damaged committed-session evidence first, then bounded pending and stale-cleanup warnings. The serialized one-toast contract remains unchanged.
