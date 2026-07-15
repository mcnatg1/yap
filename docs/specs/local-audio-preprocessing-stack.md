# Spec: Local Audio Preprocessing Stack

**Status:** Accepted design contract; desktop capture/timeline/recording foundation verified 2026-07-11, with gated Phase 5 canonical-WAV admission, PCM extraction, and loopback batch transport merged 2026-07-15
**Scope:** Desktop-side capture and deterministic preprocessing before local fallback or server upload.
**Amended by:** [ADR 0020](../adr/0020-meeting-capture-diarization-authority.md) and the [Source-Aware Diarization Design](source-aware-diarization.md).

Yap should preprocess audio locally when the work is cheap, deterministic, and useful for both the local live fallback and the server path. The server owns heavy inference and enrichment. The desktop owns capture, preparation, chunk metadata, and retryable transport packaging.

## Product Rule

```text
desktop = capture + prepare + package + local live fallback + optional anonymous speaker evidence
server  = official inference + reconciliation + purpose-authorized identity + team storage/routing
```

Local preprocessing should make server work cheaper and more reliable without turning the desktop into an all-local meeting authority. Local meeting results may contain anonymous `Unknown` and `Speaker N` labels. Named speaker identity remains server-authoritative.

## Meetily Reference

Use Meetily as a reference for edge cases and naming, not as code to copy wholesale.

| Meetily file | Useful reference | Why it matters |
|--------------|------------------|----------------|
| `frontend/src-tauri/src/audio/pipeline.rs:194` | `AudioCapture` | End-to-end capture loop and stream error handling. |
| `frontend/src-tauri/src/audio/pipeline.rs:386` | `process_audio_data` | Frame ingestion into processing pipeline. |
| `frontend/src-tauri/src/audio/pipeline.rs:680` | `AudioPipeline` | Coordinates capture, mixing, and transcription handoff. |
| `frontend/src-tauri/src/audio/pipeline.rs:944` | `AudioPipelineManager` | Start/stop lifecycle for capture pipelines. |
| `frontend/src-tauri/src/audio/vad.rs:9` | `SpeechSegment` | Segment metadata shape. |
| `frontend/src-tauri/src/audio/vad.rs:17` | `ContinuousVadProcessor` | Continuous VAD pattern with redemption/silence handling. |
| `frontend/src-tauri/src/audio/vad.rs:87` | `process_audio` | Converts sample windows into speech segments. |
| `frontend/src-tauri/src/audio/vad.rs:288` | `extract_speech_16k` | Speech-only extraction reference. |
| `frontend/src-tauri/src/audio/devices/discovery.rs:9` | `list_audio_devices` | Device enumeration UX and permission edge cases. |
| `frontend/src-tauri/src/audio/devices/configuration.rs:111` | `get_device_and_config` | Device/config resolution boundary. |
| `frontend/src-tauri/src/audio_v2/resampler.rs:9` | `DynamicResampler` | Runtime sample-rate conversion idea. |
| `frontend/src-tauri/src/audio_v2/normalizer.rs:9` | `AudioNormalizer` | Local normalization boundary. |
| `frontend/src-tauri/src/audio_v2/stream.rs:21` | `ProcessedAudio` | Packaged processed chunk shape. |
| `frontend/src-tauri/src/audio_v2/stream.rs:29` | `ModernAudioStream` | Stream wrapper idea, not a dependency target. |

Do not copy Meetily's local Whisper/Parakeet transcription router, old backend, unfinished `audio_v2` scaffolding, or its `frontend/src-tauri` layout. Yap's Tauri app lives under `desktop/src-tauri`; keeping `src-tauri` inside a folder named `frontend` would blur the desktop ownership boundary.

## Yap Current Anchors

| Yap file | Current responsibility | Next boundary |
|----------|------------------------|---------------|
| `desktop/src-tauri/src/live/runtime.rs` | Nemotron-gated CPAL mic capture, track-aware coordinator input, bounded recording/local-ASR consumers, bounded evidence/transport ports, level, resampling, local ASR, and streaming recording. | Wire evidence and server-transport consumers only after their implementations exist. |
| `desktop/src-tauri/src/live/devices.rs` | Input device listing/resolution. | Remain the device source for live and server capture. |
| `desktop/src-tauri/src/live/stream.rs` | Nemotron stream chunk constants and recognizer wrapper. | Keep ASR-specific chunking here; move transport-neutral chunk metadata elsewhere. |
| `desktop/src-tauri/src/live/recordings.rs` | Validates committed capture manifests, projects canonical history, and owns recovery/deletion of partial and committed artifacts. | Add server-job linkage without changing capture identity. |
| `desktop/src-tauri/src/audio/` | Track-aware frames, preprocessing, exact timeline gaps, independent bounded sink-port coordination, streaming recording, immutable sidecars/commits, and tested manifest contracts. | Add optional speaker inference and transport consumers without another recording contract. |
| `desktop/src-tauri/src/jobs/` | Durable imported-job lifecycle, strict Phase 5 canonical-WAV admission and immutable PCM extraction/spool, reconnect drain, cancellation outbox, retention, and verified result catalog. | Keep general media conversion, live-capture transport, and Phase 6 preprocessing separate from imported-file authority. |
| `desktop/src-tauri/src/runtime/` | Application-wide background-task lifecycle and shared setup/connector projection enums. | Keep durable jobs, connector generations, and live sessions in their domain owners. |
| `server/` | Versioned contracts plus the Phase 5 loopback durable batch service, router, and isolated Cohere pool. | Add authenticated live and production deployment only at their later gates. |

## Verified Implementation Status

Implemented and connected in the production microphone path after required Nemotron/local-ASR startup:

- Track-aware prepared frames and one ordered recording input contract: `PreparedFrame`, atomic `RevisionTransition`, and exact `Gap`.
- Callback-safe source positions, clock/configuration revisions, explicit loss gaps, and independent bounded recording and local-ASR consumers.
- Bounded-memory streaming WAV persistence with no retained-PCM duration cap.
- Immutable capture sidecar and commit publication, hash-validated catalog projection, and recover/delete handling for partial and committed recordings.

The live-capture evidence and server-transport ports are implemented and independently bounded, but their production consumers are currently `None`. Phase 5 instead admits already-canonical mono PCM16/16 kHz WAV files and extracts their PCM through `jobs/remote.rs`; it does not decode or resample general imported media and does not wire live microphone transport. Production capture does not yet run recording-only: `start_local` must construct the Nemotron stream and local-ASR adapter before it opens CPAL capture.

Deferred: live-capture upload/WSS, application authentication, persistent/external service deployment, system loopback, Opus transport, an anonymous-speaker/diarization model, long-recording and multi-worker capacity evidence, hosted production-release proof, and per-OS real-model/native hardware CI. The gated Phase 5 imported-file loopback batch path and its verified result projection exist.

Pre-release timestamp-era recordings remain untouched and unindexed. There is no migration adapter or second fixture/recording contract for them.

## Local Responsibilities

The desktop should own these before audio crosses a process or network boundary:

- Input device selection and fallback.
- Permission/preflight errors.
- Distinguish session mode, trigger gesture, session origin, and physical capture source.
- Convert channels within one physical source to mono `f32`; never collapse microphone and system loopback into one authoritative track.
- Resample to `16_000 Hz` for local fallback and the current Phase 5 imported-file batch contract; a future live contract may negotiate another supported rate explicitly.
- Compute RMS/level for UI and diagnostics.
- Optional bounded normalization/limiting that cannot clip or hide silence.
- VAD/endpointing boundaries.
- Chunk timestamps using a monotonic session clock.
- Per-track descriptors, sequence continuity, clock conversion, and explicit gap events.
- Session manifest metadata.
- Retryable upload metadata.
- Local WAV save for user playback/debugging.
- Streaming, bounded-memory meeting persistence with crash-recoverable partial artifacts.
- Optional session-scoped anonymous speaker evidence that cannot produce names or durable profiles.

## Server Responsibilities

The server owns:

- Official long-recording ASR.
- Server live ASR when connected.
- Model routing and fusion.
- Authoritative diarization, result reconciliation, and named speaker identity.
- Forced alignment when model-heavy.
- Language ID when model-heavy.
- Batch repair, reprocessing, and team storage.

## Proposed Local Module Shape

The implemented module boundary is intentionally small:

```text
desktop/src-tauri/src/audio/
  mod.rs
  session.rs      session mode, trigger mode, track plan
  frame.rs        sample format, timestamp, track-aware chunk metadata
  preprocess.rs   downmix, resample, normalize, level helpers
  vad.rs          endpointing and silence boundaries
  timeline.rs     common session clock, frames, explicit gaps
  evidence.rs     anonymous speaker evidence and bounded session clusters
  manifest.rs     session/chunk manifest serialization
```

Keep Tauri command wiring thin. Keep local ASR in `live/stream.rs`, durable imported-file preparation in `jobs/remote.rs`, and wire behavior in `server_connector/`; do not create a second recording or queue authority.

## Data Shapes

Rust-owned conceptual shape:

```rust
pub struct AudioSessionManifest {
    pub schema_version: u16,
    pub session_id: SessionId,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub started_at_utc: String,
    pub utc_offset_minutes_at_start: Option<i16>,
    pub locale_hint_bcp47: Option<String>,
    pub country_code_hint: Option<String>,
    pub preferred_languages_bcp47: Vec<String>,
    pub app_version: String,
    pub platform: String,
    pub privacy_policy_version: String,
    pub retention_expires_at_utc: Option<String>,
    pub tracks: Vec<CaptureTrackDescriptor>,
}

pub struct AudioFrame {
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub sequence: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub samples_f32_mono: Vec<f32>,
}

pub struct AudioChunkManifest {
    pub owner_namespace: OwnerNamespace,
    pub schema_version: u16,
    pub chunk_id: String,
    pub session_id: SessionId,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub track_id: TrackId,
    pub track_source: TrackSource,
    pub sequence_start: u64,
    pub sequence_end: u64,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
    pub codec: AudioCodec,
    pub content_sha256: String,
    pub audio_artifact_id: String,
    pub vad_segments: Vec<VadSegment>,
    pub gaps: Vec<AudioGap>,
    pub route: AudioRoute,
    pub degraded: bool,
}

pub enum RecordingInput {
    PreparedFrame(PreparedFrame),
    RevisionTransition(RecordingRevisionTransition),
    Gap(AudioGap),
}
```

`SessionMode` is `Dictation` or `Meeting`. `SessionOrigin` is `LiveCapture` or `ImportedFile`. Imported tracks carry `Unknown`, `Mixed`, or user-declared physical provenance. `track_id` participates in ordering, chunk IDs, and logical idempotency keys. Byte identity remains the separate `content_sha256`. Same logical key/same hash is an idempotent replay; same key/different hash is a conflict; different keys/equal hashes are valid. The local owner namespace prevents collisions on one installation; the server replaces it with the token-derived tenant/owner namespace and does not trust client ownership claims. Session builders reject foreign sessions, foreign tracks, impossible timing, and incompatible sample rates without a recorded conversion. Historical `AudioSource::{Live, Recording}` values are not reused as physical microphone/system-loopback provenance.

Session metadata uses UTC for the history/audit anchor and monotonic milliseconds for all media timing. Locale and preferred languages use BCP 47; an optional country hint uses ISO 3166-1 alpha-2 and is accepted only from explicit user/organization configuration when routing needs it. Do not infer country from IP or location. Device references are opaque and app-local; raw OS device labels remain diagnostic-only and are not uploaded by default. Processing state, retries, and transient errors remain mutable runtime/ledger data instead of rewriting this manifest.

The real-time callback reports a full handoff through a preallocated per-track atomic loss accumulator rather than the already-full event queue. The coordinator drains the first dropped source position, dropped-frame count, and loss generation through atomic swap/compare-exchange before the next accepted frame and during finalization. Updates that race the drain remain in the next generation. Drained snapshots become deterministic `Gap` events. Callback code does not allocate, block, or write to disk.

The Phase 5 imported-file transport uses canonical mono PCM16/16 kHz first. Opus can wait until the authenticated WSS/live or production upload profile proves it needs the bandwidth savings.

## VAD And Chunking

Use the current live stream default as the starting point:

- Local fallback continues to use the tuned Nemotron chunk size from ADR 0019.
- Current Phase 5 imported-file chunks are exact bounded PCM sequence ranges of
  at most one MiB. Phase 6 may align processing/transport windows to VAD hints,
  but uninterrupted speech or VAD failure must never create an unbounded chunk.
- Tail padding must avoid clipping final words.
- Silence-only chunks may be skipped locally but represented in the manifest when needed for timestamps.
- VAD failure must not delete source audio.

## Error Policy

- Device missing: recover to default input when possible.
- Device denied: block capture with a user-actionable message.
- Resampler failure: fail the session before upload/transcription.
- VAD failure: continue with unsegmented chunks and mark VAD as `error`.
- Backpressure: queue bounded chunks; if full, pause intake or fail visibly rather than dropping speech silently.
- Callback loss: emit an explicit gap; never concatenate retained samples as though no time was lost.
- Optional sink failure: recording, ASR, evidence, and transport finalize independently and compose one degraded session result.
- Manifest mismatch: reject mixed-session or mixed-track content instead of relabeling it under the caller's session.
- Server unavailable: keep local live fallback available, queue/block official recordings per the client state-machine spec.

## Out Of Scope

- No system audio capture implementation for the first source-aware plan; only the track-aware seam is in scope.
- No local Whisper/Parakeet routing.
- No local official batch transcription path.
- No local named-speaker matching or durable guest voice profiles.
- No persistent client speaker embeddings; retain audio and anonymous timelines, then recompute when authorized.
- No GPU selector in the desktop.
- No new audio crate unless current `cpal` plus existing helpers cannot meet a measured requirement.
- No `frontend/src-tauri` repo layout.

## Acceptance

- The spec for server upload can depend on a local chunk/session manifest.
- Local preprocessing is deterministic and unit-testable without a microphone.
- Live fallback still works without the server.
- Larger recordings still queue/block without a server.
- The Phase 5 loopback profile prepares and drains imported recordings through
  one durable contract and publishes only verified immutable results; exact PR
  head `4771d9be60562fa009ccecbcd3c7111b699883a5` passed its checked-head gate.
- Dictation remains independent from speaker evidence.
- Current canonical single-microphone artifacts and settings remain readable; timestamp-era recordings remain untouched and unindexed.
- Meeting capture streams to disk with bounded memory and no retained-PCM duration cap.
- Track-aware builders fail closed on cross-session, cross-track, hash, and timing conflicts.
- Local speaker output is limited to `Unknown` and session-scoped `Speaker N`.
- The first implementation can be tested with synthetic PCM frames before touching real devices.
- Existing checks still pass: `pnpm test`, `pnpm build`, and `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml`.
