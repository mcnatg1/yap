# Spec: Local Audio Preprocessing Stack

**Status:** Draft
**Scope:** Desktop-side capture and deterministic preprocessing before local fallback or server upload.
**Amended by:** [ADR 0020](../adr/0020-meeting-capture-diarization-authority.md) and the [Source-Aware Diarization Design](../superpowers/specs/2026-07-10-source-aware-diarization-design.md).

Yap should preprocess audio locally when the work is cheap, deterministic, and useful for both the local live fallback and the future server path. The server owns heavy inference and enrichment. The desktop owns capture, preparation, chunk metadata, and retryable transport packaging.

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

| Yap file | Current responsibility | Expected change |
|----------|------------------------|-----------------|
| `desktop/src-tauri/src/live/runtime.rs` | Mic capture, PCM recording, level, resampling, local stream feeding. | Split reusable preprocessing pieces when server transport needs them. |
| `desktop/src-tauri/src/live/devices.rs` | Input device listing/resolution. | Remain the device source for live and server capture. |
| `desktop/src-tauri/src/live/stream.rs` | Nemotron stream chunk constants and recognizer wrapper. | Keep ASR-specific chunking here; move transport-neutral chunk metadata elsewhere. |
| `desktop/src-tauri/src/live/recordings.rs` | Saves live WAV/transcript files. | Use manifest metadata once chunks exist. |
| `desktop/src-tauri/src/audio/` | Deterministic frame, VAD, preprocessing, and manifest scaffolding. | Add session, track, timeline-gap, and anonymous-evidence contracts before server transport. |
| `desktop/src-tauri/src/runtime/` | Future Rust RuntimeOrchestrator home. | Own route decisions and server/local job state. |
| `server/` | Future server contract and route tests. | Receive documented chunk/session format. |

## Local Responsibilities

The desktop should own these before audio crosses a process or network boundary:

- Input device selection and fallback.
- Permission/preflight errors.
- Distinguish session mode, trigger gesture, session origin, and physical capture source.
- Convert channels within one physical source to mono `f32`; never collapse microphone and system loopback into one authoritative track.
- Resample to `16_000 Hz` for local fallback and to the server-requested rate when the server contract exists.
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

Do not create this tree until implementation starts. When it does, keep it small:

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

Keep Tauri command wiring in `lib.rs` thin. Keep local ASR in `live/stream.rs`. Keep server connector code out until the contract spec is ready.

## Data Shapes

Rust-owned conceptual shape:

```rust
pub struct AudioFrame {
    pub session_id: u64,
    pub track_id: String,
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
    pub session_id: u64,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub track_id: String,
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

pub enum AudioTimelineEvent {
    TrackConfigured(TrackConfigurationRevision),
    ClockMapped(ClockMappingRevision),
    Frame(AudioFrame),
    Gap {
        session_id: u64,
        track_id: String,
        start_ms: u64,
        duration_ms: u32,
        cause: GapCause,
    },
}
```

`SessionMode` is `Dictation` or `Meeting`. `SessionOrigin` is `LiveCapture` or `ImportedFile`. Imported tracks carry `Unknown`, `Mixed`, or user-declared physical provenance. `track_id` participates in ordering, chunk IDs, and logical idempotency keys. Byte identity remains the separate `content_sha256`. Same logical key/same hash is an idempotent replay; same key/different hash is a conflict; different keys/equal hashes are valid. The local owner namespace prevents collisions on one installation; the server replaces it with the token-derived tenant/owner namespace and does not trust client ownership claims. Session builders reject foreign sessions, foreign tracks, impossible timing, and incompatible sample rates without a recorded conversion. The current `AudioSource::{Live, Recording}` migrates to a session-origin concept and must not be repurposed for microphone versus system loopback.

The real-time callback reports a full handoff through a preallocated per-track atomic loss accumulator rather than the already-full event queue. The coordinator drains the first dropped source position, dropped-frame count, and loss generation through atomic swap/compare-exchange before the next accepted frame and during finalization. Updates that race the drain remain in the next generation. Drained snapshots become deterministic `Gap` events. Callback code does not allocate, block, or write to disk.

Transport payloads can be PCM first. Opus can wait until the server WSS/upload contract proves it needs the bandwidth savings.

## VAD And Chunking

Use the current live stream default as the starting point:

- Local fallback continues to use the tuned Nemotron chunk size from ADR 0019.
- Server upload chunks use bounded wall-clock windows plus VAD boundaries. Uninterrupted speech or VAD failure must not create an unbounded chunk.
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
- Dictation remains independent from speaker evidence.
- Existing single-microphone artifacts and settings remain readable.
- Meeting capture is not limited by the current in-memory retained-PCM cap.
- Track-aware builders fail closed on cross-session, cross-track, hash, and timing conflicts.
- Local speaker output is limited to `Unknown` and session-scoped `Speaker N`.
- The first implementation can be tested with synthetic PCM frames before touching real devices.
- Existing checks still pass: `pnpm test`, `pnpm build`, and `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml`.
