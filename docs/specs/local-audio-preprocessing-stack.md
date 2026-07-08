# Spec: Local Audio Preprocessing Stack

**Status:** Draft
**Scope:** Desktop-side capture and deterministic preprocessing before local fallback or server upload.

Yap should preprocess audio locally when the work is cheap, deterministic, and useful for both the local live fallback and the future server path. The server owns heavy inference and enrichment. The desktop owns capture, preparation, chunk metadata, and retryable transport packaging.

## Product Rule

```text
desktop = capture + prepare + package + local live fallback
server  = infer + enrich + diarize + store/team route
```

Local preprocessing should make server work cheaper and more reliable without turning the desktop into an all-local meeting assistant.

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
| `desktop/src-tauri/src/runtime/` | Future Rust RuntimeOrchestrator home. | Own route decisions and server/local job state. |
| `server/` | Future server contract and route tests. | Receive documented chunk/session format. |

## Local Responsibilities

The desktop should own these before audio crosses a process or network boundary:

- Input device selection and fallback.
- Permission/preflight errors.
- Convert input samples to mono `f32`.
- Resample to `16_000 Hz` for local fallback and to the server-requested rate when the server contract exists.
- Compute RMS/level for UI and diagnostics.
- Optional bounded normalization/limiting that cannot clip or hide silence.
- VAD/endpointing boundaries.
- Chunk timestamps using a monotonic session clock.
- Session manifest metadata.
- Retryable upload metadata.
- Local WAV save for user playback/debugging.

## Server Responsibilities

The server owns:

- Official long-recording ASR.
- Server live ASR when connected.
- Model routing and fusion.
- Diarization and speaker identity.
- Forced alignment when model-heavy.
- Language ID when model-heavy.
- Batch repair, reprocessing, and team storage.

## Proposed Local Module Shape

Do not create this tree until implementation starts. When it does, keep it small:

```text
desktop/src-tauri/src/audio/
  mod.rs
  frame.rs        sample format, timestamp, chunk metadata
  preprocess.rs   downmix, resample, normalize, level helpers
  vad.rs          endpointing and silence boundaries
  manifest.rs     session/chunk manifest serialization
```

Keep Tauri command wiring in `lib.rs` thin. Keep local ASR in `live/stream.rs`. Keep server connector code out until the contract spec is ready.

## Data Shapes

Rust-owned conceptual shape:

```rust
pub struct AudioFrame {
    pub session_id: u64,
    pub sequence: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub samples_f32_mono: Vec<f32>,
}

pub struct AudioChunkManifest {
    pub session_id: u64,
    pub chunk_id: String,
    pub sequence_start: u64,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
    pub codec: AudioCodec,
    pub vad: VadDecision,
    pub route: AudioRoute,
}
```

Transport payloads can be PCM first. Opus can wait until the server WSS/upload contract proves it needs the bandwidth savings.

## VAD And Chunking

Use the current live stream default as the starting point:

- Local fallback continues to use the tuned Nemotron chunk size from ADR 0019.
- Server upload chunks should prefer stable wall-clock windows plus VAD boundaries.
- Tail padding must avoid clipping final words.
- Silence-only chunks may be skipped locally but represented in the manifest when needed for timestamps.
- VAD failure must not delete source audio.

## Error Policy

- Device missing: recover to default input when possible.
- Device denied: block capture with a user-actionable message.
- Resampler failure: fail the session before upload/transcription.
- VAD failure: continue with unsegmented chunks and mark VAD as `error`.
- Backpressure: queue bounded chunks; if full, pause intake or fail visibly rather than dropping speech silently.
- Server unavailable: keep local live fallback available, queue/block official recordings per the client state-machine spec.

## Out Of Scope

- No system audio capture for MVP.
- No local Whisper/Parakeet routing.
- No local official batch transcription path.
- No GPU selector in the desktop.
- No new audio crate unless current `cpal` plus existing helpers cannot meet a measured requirement.
- No `frontend/src-tauri` repo layout.

## Acceptance

- The spec for server upload can depend on a local chunk/session manifest.
- Local preprocessing is deterministic and unit-testable without a microphone.
- Live fallback still works without the server.
- Larger recordings still queue/block without a server.
- The first implementation can be tested with synthetic PCM frames before touching real devices.
- Existing checks still pass: `pnpm test`, `pnpm build`, and `cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml`.
