# Source-Aware Diarization Design

**Status:** Accepted design; client capture foundation implemented and verified 2026-07-11; model-specific diarization remains deferred
**Date:** 2026-07-10
**Scope:** Track-aware client audio contracts, local anonymous speaker evidence, and server-authoritative reconciliation for Yap meeting sessions.
**Decision:** [ADR 0020](../../adr/0020-meeting-capture-diarization-authority.md)

## Problem

Yap's implemented live path records one microphone stream through the source-aware coordinator after Nemotron and its local-ASR adapter start successfully. Track-aware prepared frames, atomic configuration/clock revisions, and exact gaps fan out through independent bounded ports. Production wires recording and local ASR; the evidence and server-transport ports are implemented but their consumers are `None`. The recording sink streams to disk and publishes an immutable capture sidecar and commit; recovery and deletion operate on that canonical lineage. System loopback, server transport, speaker inference, and ASR-independent production capture remain future work on the same contract.

The existing diarization ADRs also disagree about ownership and algorithms. This design establishes the contract before selecting or integrating a heavier diarization model.

## Goals

- Preserve dictation behavior and latency when speaker processing is absent, slow, or broken.
- Represent session mode, trigger gesture, physical capture source, model-local speaker slot, session speaker, and durable identity independently.
- Preserve timestamp-aligned source tracks and explicit gaps.
- Keep meeting recording crash-recoverable and streaming so retained source audio is available for reconciliation without a retained-PCM duration cap.
- Produce useful local `Unknown` and `Speaker N` results without storing durable guest biometrics.
- Let a server reprocess retained audio and publish revisioned authoritative results.
- Reuse existing dependencies for the first measurable baseline.
- Make failure states, resource bounds, privacy, and testing requirements explicit.

## Non-goals

- Implement Windows system loopback in the first plan.
- Implement the server connector, server diarization service, Entra enrollment, or durable voice-profile store in the first plan.
- Put diarization on the dictation critical path.
- Persist local speaker embeddings across sessions.
- Select EEND-VC or MS-SphereVBx before comparative benchmarks.
- Store audio or transcript bytes in SQLite.

## Current Anchors

| File | Current role | Design implication |
|------|--------------|--------------------|
| `desktop/src-tauri/src/live/runtime.rs` | Nemotron-gated CPAL microphone adapter, source-aware coordinator, bounded recording/local-ASR consumers, bounded evidence/transport ports, and streaming recording | Add future source/transport/evidence consumers without changing dictation behavior. |
| `desktop/src-tauri/src/audio/frame.rs` | Track-aware prepared-frame, exact-gap, chunk, replay-key, and content-identity contracts | Keep this as the canonical media contract. |
| `desktop/src-tauri/src/audio/manifest.rs` | Strict session/chunk envelope builders with ownership, content identity, and timeline validation | Extend only through current schema decisions; do not add a compatibility adapter. |
| `desktop/src-tauri/src/audio/preprocess.rs` | Deterministic mono conversion, resampling, RMS | Reuse per track. |
| `desktop/src-tauri/src/audio/vad.rs` | Deterministic energy VAD scaffolding | Keep VAD advisory; never use it to erase source audio. |
| `desktop/src-tauri/src/live/recordings.rs` | Hash-valid committed catalog, immutable transcript revisions, partial recovery, and deletion | Link future server jobs to committed artifacts without changing capture completeness. |
| `desktop/src-tauri/Cargo.toml` | Already depends on `sherpa-onnx` | Use its speaker APIs for the first benchmark instead of adding another runtime. |

## Domain Model

The implementation plan will refine names against existing code, but these concepts are normative:

```rust
pub enum SessionMode {
    Dictation,
    Meeting,
}

pub enum SessionOrigin {
    LiveCapture,
    ImportedFile,
}

pub enum TriggerMode {
    PushToTalk,
    Toggle,
}

pub enum CaptureSource {
    Microphone,
    SystemLoopback,
}

pub enum ImportedTrackProvenance {
    Unknown,
    Mixed,
    UserDeclared(CaptureSource),
}

pub enum TrackSource {
    Captured(CaptureSource),
    Imported(ImportedTrackProvenance),
}

pub struct CaptureTrackDescriptor {
    pub track_id: TrackId,
    pub source: TrackSource,
    pub device_id: Option<String>,
    pub original_sample_rate_hz: u32,
    pub original_channels: u16,
}

pub struct SessionMetadata {
    pub session_id: SessionId,
    pub mode: SessionMode,
    pub origin: SessionOrigin,
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

pub enum RecordingInput {
    PreparedFrame(PreparedFrame),
    RevisionTransition(RecordingRevisionTransition),
    Gap(AudioGap),
}

pub struct RecordingRevisionTransition {
    pub configuration: TrackConfigurationRevision,
    pub clock_mapping: ClockMappingRevision,
}

pub struct TrackConfigurationRevision {
    pub track_id: TrackId,
    pub revision: u32,
    pub effective_at_ms: u64,
    pub device_id: Option<String>,
    pub original_sample_rate_hz: u32,
    pub original_channels: u16,
}

pub struct ClockMappingRevision {
    pub track_id: TrackId,
    pub revision: u32,
    pub source_position_frames: u64,
    pub session_time_ms: u64,
}

pub struct PreparedFrame {
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub sequence: u64,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
    pub samples: Arc<[f32]>,
}
```

`TriggerMode` carries the durable gesture meaning while serialized live settings remain backward compatible. `SessionMode` says whether the workflow is dictation or a meeting. `SessionOrigin` says whether audio was captured live or imported. An imported file does not claim microphone or system provenance unless the user explicitly supplies it, and mixed imports remain `Mixed`. Historical `AudioSource::{Live, Recording}` values are not reused as physical source provenance.

`started_at_utc` anchors the session for history and audit; all audio, diarization, and word timing uses the monotonic session timeline. Locale, country, and language values are normalized hints, not inferred identity: BCP 47 for locale/language and ISO 3166-1 alpha-2 for country. `country_code_hint` is collected only from an explicit user or organization setting when routing actually needs it; Yap does not derive it from IP address or device location. A track's `device_id` is an opaque app-local configuration reference; raw OS device labels are diagnostic data and are not uploaded by default. Mutable processing/retry state belongs in the runtime or durable job ledger, not the immutable capture manifest.

## Capture And Fan-Out

One capture coordinator owns the monotonic session clock and accepts input from capture adapters. The production CPAL microphone is the first adapter. A future WASAPI loopback adapter uses the same contract. A device, format, or source-clock change applies one atomic configuration/clock revision transition before subsequent frames; conversion metadata remains replayable instead of being inferred from callback counts.

Preprocessing is per track. The coordinator exposes independent bounded sink ports. Current production supplies the recording and local-ASR consumers; speaker-evidence and server-transport consumers remain unwired:

| Sink | Required behavior under pressure or failure |
|------|---------------------------------------------|
| Recording | Highest priority after capture; persists gaps and never waits for ASR. |
| Local ASR | May degrade or stop without ending recording. |
| Speaker evidence | May lag, skip low-priority analysis, or mark degraded without ending recording or ASR. |
| Server transport | May spool/retry later; cannot own the only source copy. |

The audio callback never blocks on inference or disk. A full bounded handoff cannot also be the only path used to report that it is full. Each track therefore owns a preallocated atomic loss accumulator: first dropped source position, dropped-frame count, and monotonic loss generation. The callback updates it without allocation or waiting. The coordinator drains it with atomic swap/compare-exchange before the next accepted frame and at finalization; updates racing the drain remain in the next generation. Drained snapshots become deterministic `Gap` events before later audio. Only contiguous losses with the same cause may coalesce. Stop/finalization closes each configured sink independently and composes their outcomes into one session result.

## Speaker Evidence And Attribution

Speaker evidence is model output with provenance, not identity:

```rust
pub struct SpeakerEvidence {
    pub track_id: TrackId,
    pub interval: TimeInterval,
    pub local_slot_id: Option<LocalSlotId>,
    pub embedding_model: ModelRevision,
    pub quality: EvidenceQuality,
    pub confidence: Option<f32>,
}

pub enum SpeakerAttribution {
    Unknown,
    SessionSpeaker(SessionSpeakerId),
    Named(NamedSpeakerAssertion),
}
```

The client emits only `Unknown` or `SessionSpeaker`. Internally, anonymous clustering uses `Unknown`, `Candidate`, and `StableAnonymous`. `Candidate` remains rendered as `Unknown`; it is never exposed as a flickering `Speaker N`. Promotion requires calibrated quality, score, runner-up margin, and repeated or cumulative evidence. A published `SessionSpeaker` remains stable within its immutable result revision. Demotion, merge, or split is expressed by a later revision. `Named` is accepted from a server result with identity, consent/profile revision, model/calibration revision, confidence, and provenance.

A user-supplied contact or text label is a separate display annotation with `user` provenance. It does not change `SpeakerAttribution`, claim a biometric match, or authorize profile creation. This separation lets a transcript display "Alex" while the machine result remains session-scoped `Speaker 2`.

Evidence shorter than 1.6 seconds is weak. By itself, it may receive a temporally smoothed anonymous assignment but cannot establish a stable session cluster or authorize any profile update. Weak repeated turns can accumulate into an anonymous cluster. No duration alone guarantees identity.

Session cluster state is bounded to centroid, weighted evidence count, cumulative clean duration, confidence, last-seen time, and at most 20 transient exemplars. The product target is 32 session speakers and the safety ceiling is 64. Exceeding the ceiling yields unknown attribution and a reprocessing marker.

## Baseline And Promotion Gates

Slice 8c uses existing `sherpa-onnx` speaker embedding and offline diarization APIs as a baseline. Model artifacts remain an explicit optional download and must pass license review. No model is bundled merely because the API exists. The baseline must pass every applicable absolute gate below before it ships; these are not challenger-only gates.

SphereVBx-PF is the first clustering challenger. EEND-VC plus MS-SphereVBx is the overlap-aware challenger. A challenger is promoted only when all applicable gates pass:

| Gate | Initial target |
|------|----------------|
| No-collar DER with overlap scored | At or below 20% on the approved fixture suite |
| Speaker-count mean absolute error | At or below 0.5 |
| Named identity precision | At least 99.5% when identity work begins |
| Open-set false-name rate | At most 0.1% when identity work begins |
| Client p95 CPU increase | Below 5 percentage points while anonymous meeting evidence is active on reference hardware |
| Client RSS increase | Below 150 MB while anonymous diarization is active |
| Local-ASR latency regression | Below 10% |
| Audio callback drops | Zero under the supported-load test |
| Final server diarization | RTF at or below 0.25 at target concurrency |

The EEND challenger must also improve macro DER by at least 10% relative and overlap DER by at least 20% relative over the accepted baseline. Other challengers must not regress any absolute gate and must document the product-relevant improvement that justifies added complexity. Thresholds and targets may change only through recorded benchmark evidence and a reviewed spec amendment.

## Result Revisions

Every speaker-bearing result has a monotonically increasing revision and provenance:

```text
local provisional r1
  -> local reconciled r2
  -> server authoritative r3
  -> user-corrected r4
```

Reprocessing appends a result. It never mutates raw audio or silently replaces a user correction. A later server result may be presented as a proposed revision when manual labels exist.

`Unknown` remains valid in any revision. A result is `partial` when source audio has gaps, is truncated, or was not fully uploaded.

Timestamped diarization is normative at two levels:

```rust
pub struct SpeakerTurn {
    pub turn_id: TurnId,
    pub start_ms: u64,
    pub end_ms: u64,
    pub attribution: SpeakerAttribution,
    pub confidence: f32,
    pub supporting_track_ids: Vec<TrackId>,
    pub overlap_group_id: Option<OverlapGroupId>,
}

pub struct AlignedWord {
    pub word_index: u32,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub turn_id: Option<TurnId>,
    pub attribution: SpeakerAttribution,
    pub confidence: Option<f32>,
}

pub struct SpeakerResultRevision {
    pub session_id: SessionId,
    pub revision: u64,
    pub authority: ResultAuthority,
    pub created_at_utc: String,
    pub capture_manifest_sha256: String,
    pub previous_result_sha256: Option<String>,
    pub status: ResultStatus,
    pub language: Option<LanguageDecision>,
    pub speaker_turns: Vec<SpeakerTurn>,
    pub aligned_words: Vec<AlignedWord>,
    pub model_provenance: Vec<ModelRevision>,
}
```

Intervals are end-exclusive `[start_ms, end_ms)` on the common monotonic session timeline. Speaker turns may overlap; overlap is not flattened into one guessed speaker. Segment-level turns are available as soon as diarization produces them. Word-level speaker timestamps are added after raw-text forced alignment and majority-overlap intersection. Polished text may reference aligned raw-word indices but never invents its own timings. A result can omit `aligned_words` while alignment is pending or unavailable without losing the timestamped speaker-turn timeline.

## Persistence And Reconnect

Source audio, transcript text, and human-readable speaker timelines remain inspectable files. An immutable capture sidecar stores track descriptors, configuration and clock-mapping revisions, gaps, and source artifact hashes. Result revisions are separate immutable artifacts so later reconciliation cannot invalidate the committed capture.

The capture sidecar/commit protocol, hash-validated catalog, partial recovery, and deletion are implemented for the production microphone path. Pre-release timestamp-era recordings remain untouched and unindexed, with no migration adapter or alternate reader.

Local embeddings and centroids are memory-only. They are discarded after local finalization. Server reconciliation recomputes them from retained audio.

Before automatic upload, retry, or reconnect drain ships, pending jobs move from the frontend queue shell to a Rust-owned SQLite ledger as required by the existing client storage design. SQLite stores job and revision metadata, never WAV/Opus bytes, transcript bodies, credentials, or embeddings.

### Recording commit protocol

A session is complete only when a valid commit manifest has been published:

1. The recording sink appends audio and capture-timeline events to private temporary artifacts and flushes at bounded intervals.
2. Finalization drains the loss accumulator, closes producers, and flushes the audio.
3. The sink computes the audio hash, flushes an immutable capture sidecar that names track/configuration/clock/gap revisions, and atomically moves both finalized artifacts into place.
4. A small commit manifest naming and hashing the final artifacts is written through a temporary file and atomically published last.
5. Restart treats missing, malformed, or hash-invalid commit manifests as partial recovery candidates. A partial artifact can be recovered, retried, exported, or deleted, but cannot appear as a completed recording.

This protocol is implemented. Recording input streams through the writer; complete publication requires the immutable sidecar and commit, while failed or interrupted publication remains explicitly partial and recoverable.

The implementation uses the strongest available file flush and atomic-replace primitives on each platform and documents any residual power-loss window. Fault-injection tests stop the process before and after every numbered boundary. A future SQLite transaction may reference a committed artifact or explicitly register a partial artifact; it cannot promote one implicitly.

Each transcript or future speaker result revision is written as its own immutable artifact through temporary-write, flush, and atomic-publish. It references the capture-manifest hash, its revision number, provenance, and optionally the prior result hash. Transcript revisions are implemented; speaker-result production remains deferred. A rebuildable result index may point to the latest verified revision, but it is not part of capture completeness. Crashing during result publication leaves the committed recording valid and the unfinished result absent or partial; it never rewrites the capture manifest.

### Replay identity

Logical idempotency and byte identity are independent:

```text
ChunkKey = (schema_version, owner_namespace, session_id, track_id, sequence_start, sequence_end)
ContentIdentity = content_sha256
```

`owner_namespace` is local-only before upload and becomes the server-derived `(tenant_id, owner_subject_id)` for server durability. Same key plus same hash is an idempotent replay. Same key plus a different hash is a conflict and fails closed. Different keys with the same hash are allowed unless an explicit artifact-level deduplication policy applies. Server acknowledgements record the accepted key, hash, object ID, and offset transactionally before the client advances its ledger.

Builders reject mixed sessions, track mismatches, incompatible sample rates without a conversion record, duplicate logical keys with different content, and impossible timing.

## Contacts And Privacy

Future Yap-local or OS contact integration is a metadata feature:

```text
Contact
  opaque OS reference
  display-name snapshot
  aliases
  organization/email metadata selected by the user
  no voice embedding
```

Users may label a session speaker with a contact. That mapping is stored with the transcript and can improve search and display. Calendar or roster context may suggest candidates, but the UI must distinguish a context suggestion from a voice match.

OS contact access is opt-in. Revocation stops refresh and removes imported contact metadata; a label the user deliberately wrote into a retained transcript may remain under the transcript's own retention policy. Contact-to-principal linkage is explicit. Name, alias, or email similarity cannot turn a contact suggestion into an identity assertion.

Automatic cross-session voice matching requires an independently enrolled profile. Enrollment is explicit and separate from sign-in, contact import, meeting attendance, and transcript renaming. Guest and non-enrolled speakers remain anonymous across sessions unless the user labels each result manually.

After reconnect, the server may propose named revisions for retained sessions when it matches an enrolled profile. The proposal includes identity and profile provenance and respects existing user corrections. A manual contact label remains display metadata and is never treated as enrollment or profile-update evidence.

Pseudonymous, encrypted, or local-only embeddings remain personal data when they can re-identify someone. The default therefore follows data minimization and storage limitation: retain labels and source audio under their existing policy, discard derived guest embeddings, and recompute when authorized processing is needed.

Recorded audio remains personal data and can be reprocessed into biometric evidence. Recording notice, access, retention, and deletion are separate requirements; deleting an embedding does not anonymize the corresponding audio.

### Data lifecycle defaults

| Data | Default | Required control |
|------|---------|------------------|
| Session embeddings, centroids, exemplars | Memory only, through finalization | Intentionally non-serializable and omitted/redacted from logs and debug output |
| Incomplete recording artifacts | At most 24 hours | Recover/delete UI plus automatic expiry; never listed as complete |
| Pending upload job and its private source copy | Seven days | Retry status, visible expiry, cancellation, and deletion |
| Completed meeting audio, transcript, timeline, and contact labels | Thirty days unless a visible user/org policy selects another lawful period | Expiry is inspectable; delete invalidates jobs and removes linked artifacts/indexes |
| Imported OS-contact cache | At most 24 hours since the last authorized refresh | Permission withdrawal stops refresh and removes the imported cache |
| User-created Yap contact | Until explicit deletion or account deletion, with an annual stale-contact review | Export/delete controls; never contains an embedding |
| Enrolled voice profile | Disabled until a finite deployment expiry or review period exists | Explicit enrollment, separate adaptation grant, withdrawal, export/audit, and deletion SLA |
| Revocation/deletion tombstone | Deployment audit policy; contains no biometric vector | Restored replicas/backups must consult it before making data active |

The values above are privacy-preserving product defaults, not legal advice. A deployment can shorten or lawfully replace them but cannot silently convert them to perpetual retention. Backup/replica deletion may complete asynchronously under a documented SLA; matching eligibility and queued use stop immediately.

### Consent and tenant authority

Enrollment, matching, and adaptation are distinct purposes. A durable grant records `(tenant_id, subject_id, grant_id, purpose, notice_text_version, legal_basis_code, basis_record_ref, consent_text_version?, granted_at, revoked_at, revocation_epoch)`. Every match and named-result publication transactionally rechecks current enrollment and matching grants and epochs. Every adaptation additionally rechecks a separate adaptation grant plus independent authorization; a model prediction cannot authorize its own update.

Profile adaptation has its own logical key: `(tenant_id, subject_id, model_id, model_revision, source_session_id, source_result_revision, authorization_id)`, plus an evidence hash. Same key/same hash is idempotent; same key/different hash fails closed. The profile revision and current purpose-grant/revocation epochs are checked and committed in one transaction, so upload or result replay cannot apply the same evidence twice.

The server derives `(tenant_id, owner_subject_id)` from the validated token and ignores client-supplied ownership claims. That namespace is part of every server job, chunk key, result revision, profile lookup, object-store key, and audit event. Named assertions require session, profile, active purpose grants, and identity provenance from the same tenant. Two tenants may use identical session IDs, hashes, and revision numbers without collision.

Client transient embedding and exemplar types must not implement ordinary persistence serialization. Debug/log representations expose counts and model revisions, never vector values. Normal completion, cancellation, crash recovery, and restart tests scan sidecars, SQLite, temporary artifacts, and logs to prove that no derived guest biometric was persisted.

## Error And Recovery Contract

- Capture permission denial prevents only the affected track.
- Microphone failure during dictation is visible and ends that capture cleanly.
- Optional system-loopback failure degrades a meeting without destroying the microphone track.
- VAD failure keeps unsegmented audio and marks VAD unavailable.
- Speaker-runtime failure preserves recording and ASR and marks attribution unavailable.
- ASR failure preserves recording and speaker evidence.
- Lost server acknowledgements retry the same content identity.
- Restart resumes jobs from the Rust ledger after that phase ships.
- Stale server events cannot overwrite a newer result revision.
- Consent withdrawal advances the revocation epoch, excludes every profile revision from queued and in-flight matching, and triggers the configured deletion workflow.
- Backup or replica restore cannot reactivate a recording or profile covered by a deletion tombstone.

## Test Strategy

### Contract tests

- Session mode, trigger mode, and capture source remain orthogonal.
- Track IDs participate in ordering, hashes, chunk IDs, and idempotency keys.
- Device/format changes and clock mappings are revisioned at deterministic timeline positions.
- Mixed-session frames and foreign chunks fail closed.
- Gaps, overlaps, out-of-order frames, duplicate content, and hash conflicts are deterministic.
- A saturated audio handoff still emits the exact dropped interval through the reserved loss accumulator.
- Callback updates racing an accumulator drain survive in the next loss generation.
- Same logical key/same hash replays; same key/different hash fails; same hash/different key remains distinct.
- Current canonical single-microphone settings and artifacts remain readable; pre-release timestamp-era recordings remain untouched and unindexed.

### Runtime tests

- Coordinator/port tests prove recording continues with ASR absent, stalled, or crashed; current production startup does not yet exercise ASR-absent capture.
- Speaker evidence continues with transcript text absent.
- Evidence backpressure does not block callback or recording.
- Stop finalizes each sink once and reports composed degradation.
- A crash during streaming persistence leaves a recoverable partial artifact and never an apparently complete session.
- Fault injection at every commit-protocol boundary produces either a verified complete session or an explicit partial artifact.
- A crash while publishing a result revision cannot invalidate or rewrite the committed capture.
- Four-hour synthetic sessions remain bounded.

### Diarization tests

- One speaker, two speakers, short interjections, overlapping speech, noise, echo leakage, late arrivals, and more than four global speakers.
- Unknown remains unknown when evidence is insufficient.
- Candidate evidence remains visually unknown; repeated qualified evidence may become a stable `Speaker N` but never a name.
- Speaker labels remain stable within one result revision.
- DER, JER, speaker-count error, short-turn recall, latency, CPU, and RSS are recorded.
- The baseline itself passes the absolute accuracy/resource gates before release.

### Privacy and identity tests

- Contact import produces no voice profile.
- Transcript renaming produces no enrollment.
- Unenrolled, deleted, expired, cross-tenant, and incompatible-model profiles cannot match.
- A prediction cannot authorize its own profile update.
- Replaying a result cannot apply one authorized profile adaptation more than once.
- Enrollment, matching, and adaptation grants are independently enforced at every model revision; withdrawing matching alone makes the retained profile non-matchable.
- Withdrawal during an in-flight match or adaptation removes active eligibility before publication.
- Tenant-derived ownership isolates jobs, chunks, results, profiles, and object keys even when client identifiers and hashes collide.
- Withdrawal removes active matching eligibility immediately and purges caches, replicas, and backups according to policy.
- Restoring a pre-deletion backup cannot reactivate a deleted profile or recording.
- Logs, sidecars, temporary artifacts, and SQLite contain no transient embedding or exemplar values after normal or crashed sessions.
- Server result revisions never overwrite manual labels without acceptance.

## Phased Delivery

### Foundation slice F1: Contract and manifest correctness (implemented)

- Add track-aware session, frame, gap, chunk, evidence, and revision types.
- Add strict builder validation, logical replay keys, and separate content identity.
- Keep production microphone behavior unchanged.

### Foundation slice F2: Independent capture sinks and durable recording (implemented)

- Extract the current CPAL microphone adapter.
- Add the monotonic timeline, revisioned track/clock events, and callback-safe explicit gaps.
- Separate recording and local-ASR lifecycle ownership, and add bounded speaker-evidence/server-transport ports; their production consumers remain deferred.
- Stream microphone audio to a crash-recoverable temporary artifact with bounded memory, then finalize through the commit-manifest protocol.
- Remove the retained-PCM duration limitation for meeting sessions without allowing unbounded memory growth.
- Persist an immutable single-track capture sidecar and separate result revisions while preserving current WAV/TXT playback.

### Deferred Phase 8 slice: Local anonymous baseline

- Add optional speaker-model download state.
- Benchmark a commercially usable `sherpa-onnx` embedding model.
- Implement unknown/candidate/stable-anonymous clustering and result revisions.
- Persist only the anonymous timeline and provenance; discard embeddings.

### Subsequent plans

- Canonical Phases 3–5: Rust-owned SQLite reconnect ledger and server transport.
- Server authoritative diarization and purpose-authorized identity.
- OS contacts and roster suggestions without biometrics.
- Windows system-loopback capture.
- SphereVBx-PF and EEND/MS-SphereVBx benchmark challengers.

## Acceptance For The Client Foundation Plan

- [x] Dictation behavior and existing serialized settings remain backward compatible.
- [ ] Production microphone capture can record without constructing local ASR. Current production still requires Nemotron stream and local-ASR adapter construction before CPAL capture.
- [x] Long capture streams to disk with bounded memory and recoverable partial state; there is no retained-PCM duration cap.
- [x] Audio drops are explicit timeline gaps.
- [x] Gap reporting still works when the ordinary callback queue is saturated.
- [x] Manifest builders reject cross-session and cross-track contamination.
- [x] Recording completion requires a valid commit manifest; crash states remain partial.
- [x] Logical idempotency keys and byte hashes obey the replay matrix.
- [x] No speaker model or embedding runtime was added by the foundation plan.
- [x] Recording/local-ASR queues and evidence/transport port contracts are independently bounded; production evidence/transport consumers remain `None`.
- [x] No new inference framework was added.
- [x] Rust, frontend unit, Playwright, and native WDIO checks are green; deterministic Rust contracts precede real-device evidence.

Still deferred: the Rust-owned SQLite server-job ledger; connector/upload/WSS/auth/inference; system loopback; Opus transport; an anonymous-speaker/diarization model; a real WER/model benchmark; release packaging; and native hardware CI smoke.

## Acceptance For The Phase 8 Local Baseline Plan

- Local attribution cannot produce a name or persist an embedding.
- Candidate speaker state is hidden until stable, and the baseline passes the absolute release gates.
- All per-speaker state is bounded by the configured product and safety ceilings.
- The licensed fixture manifest, RTTM annotations, and reference-hardware benchmark report exist before a model is promoted.
