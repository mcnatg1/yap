# Local Audio Preprocessing Stack Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract deterministic local audio preparation from the live runtime into a small shared Rust module that can serve local live fallback now and produce local capture envelopes that Phase 3 server contracts can map later.

**Architecture:** `desktop/src-tauri/src/audio/` owns transport-neutral frames, preprocessing helpers, simple VAD decisions, and local capture envelopes. `desktop/src-tauri/src/live/runtime.rs` keeps capture/session ownership and calls the shared helpers. `desktop/src-tauri/src/live/stream.rs` keeps Nemotron-specific ASR chunking. Server connector code remains out of this plan. The module shape comes from `docs/specs/local-audio-preprocessing-stack.md`; changing it requires updating the spec first.

**Tech Stack:** Rust, Tauri 2, `cpal`, existing in-process audio helpers, `serde`, Cargo unit tests. No new audio crate unless a measured failure proves the current stack cannot meet the requirement.

## Global Constraints

- Desktop responsibility is capture, prepare, package, and local live fallback.
- Server responsibility remains official inference, enrichment, diarization, model routing, team storage, heavy repair, and authoritative API/WSS/job contract shapes.
- Do not add system audio capture for MVP.
- Do not add local Whisper, Parakeet, Cohere, Moonshine, or model fusion routing.
- Do not implement local official batch transcription.
- Do not add a GPU selector in the desktop.
- Do not create server connector code or authoritative server route enums until the API/WSS contract spec is ready.
- Do not change the repository layout to `frontend/src-tauri`.
- Preserve the current live fallback behavior while moving reusable pure functions.
- Make local preprocessing deterministic and unit-testable without a microphone.

---

## Task 1: Create The Shared Audio Module

**Files**

- `desktop/src-tauri/src/audio/mod.rs`
- `desktop/src-tauri/src/audio/frame.rs`
- `desktop/src-tauri/src/audio/preprocess.rs`
- `desktop/src-tauri/src/audio/vad.rs`
- `desktop/src-tauri/src/audio/manifest.rs`
- `desktop/src-tauri/src/lib.rs`

**Interfaces**

Module exports:

```rust
pub mod frame;
pub mod manifest;
pub mod preprocess;
pub mod vad;
```

Add the module to the crate root:

```rust
mod audio;
```

**Steps**

- [ ] Create `audio/mod.rs` with only module declarations and focused re-exports used by `live/runtime.rs`.
- [ ] Keep all Tauri command wiring out of `audio/`.
- [ ] Keep ASR model code out of `audio/`.
- [ ] Keep server URL, auth, upload, and queue code out of `audio/`.
- [ ] Add unit-test modules inside each audio file rather than creating new integration test scaffolding.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio
```

---

## Task 2: Define Frame And Manifest Data Shapes

**Files**

- `desktop/src-tauri/src/audio/frame.rs`
- `desktop/src-tauri/src/audio/manifest.rs`

**Interfaces**

Add frame, local purpose, and capture-envelope types:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFrame {
    pub session_id: u64,
    pub sequence: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    PcmS16Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioPurpose {
    LocalFallback,
    CaptureEnvelope,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub kind: crate::audio::vad::VadKind,
    pub rms: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioChunkEnvelope {
    pub session_id: u64,
    pub chunk_id: String,
    pub sequence_start: u64,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
    pub codec: AudioCodec,
    pub vad_segments: Vec<VadSegment>,
    pub purpose: AudioPurpose,
    pub retry: RetryMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryMetadata {
    pub idempotency_key: String,
    pub attempt: u16,
    pub max_attempts: u16,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSessionEnvelope {
    pub session_id: u64,
    pub source: AudioSource,
    pub started_at_ms: u64,
    pub sample_rate_hz: u32,
    pub chunks: Vec<AudioChunkEnvelope>,
    pub degraded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource {
    Live,
    Recording,
}
```

Helpers:

```rust
impl AudioFrame {
    pub fn duration_ms_from_samples(sample_count: usize, sample_rate_hz: u32) -> u32;
}

impl AudioChunkEnvelope {
    pub fn from_frames(
        session_id: u64,
        sequence_start: u64,
        frames: &[AudioFrame],
        codec: AudioCodec,
        vad_segments: Vec<VadSegment>,
        purpose: AudioPurpose,
    ) -> Option<Self>;
}
```

**Steps**

- [ ] Use monotonic session-relative `start_ms`; do not use wall-clock time in these types.
- [ ] Use `u64` for `session_id` and `sequence` to match live session ownership.
- [ ] Keep `AudioFrame` metadata-only in runtime paths. Do not retain large mono sample vectors in manifests because live WAV PCM is already buffered in memory.
- [ ] If a pure unit test needs samples, create local test fixtures rather than adding sample vectors to the manifest-facing type.
- [ ] Use `AudioPurpose` only as local capture metadata. It must not replace existing `LiveRoute`, `JobRoute`, or `RecordingRoute` state-machine enums.
- [ ] Do not define `serverLive`, `serverBatch`, or queued route variants in this module; Phase 3 server OpenAPI/WSS docs own those contract names.
- [ ] Generate `chunk_id` deterministically as `"{session_id}-{sequence_start}-{duration_ms}"`.
- [ ] Generate `RetryMetadata.idempotency_key` deterministically from session id, sequence start, and chunk id.
- [ ] Keep `AudioCodec` to `PcmS16Le` in this plan; add Opus only when the server contract requires it.
- [ ] Add tests for duration math, empty frame handling, retry metadata, and manifest serialization names.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml audio::
```

---

## Task 3: Move Pure Preprocessing Helpers Out Of Live Runtime

**Files**

- `desktop/src-tauri/src/audio/preprocess.rs`
- `desktop/src-tauri/src/live/runtime.rs`

**Interfaces**

Move these helpers from `live/runtime.rs` into `audio/preprocess.rs`:

```rust
pub fn downmix_to_mono(samples: &[f32], channels: usize) -> Vec<f32>;
pub fn f32_to_i16_le_bytes(samples: &[f32]) -> Vec<u8>;
pub fn rms_level(samples: &[f32]) -> f32;

pub struct AudioLevelNormalizer {
    // fields moved from LiveAudioLevelNormalizer
}

impl AudioLevelNormalizer {
    pub fn new() -> Self;
    pub fn normalized_level(&mut self, rms: f32) -> f32;
}

pub struct LinearResampler {
    // fields moved unchanged
}

impl LinearResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self;
    pub fn push(&mut self, input: &[f32]) -> Vec<f32>;
}
```

**Steps**

- [ ] Move `downmix_to_mono`, `f32_to_i16_le_bytes`, `rms_level`, `LiveAudioLevelNormalizer`, `LinearResampler`, and `mix` into `audio/preprocess.rs`.
- [ ] Rename `LiveAudioLevelNormalizer` to `AudioLevelNormalizer` in the shared module.
- [ ] Leave a local alias in `live/runtime.rs` only if it materially reduces churn:

```rust
use crate::audio::preprocess::AudioLevelNormalizer as LiveAudioLevelNormalizer;
```

- [ ] Move the existing unit tests with the functions.
- [ ] Update `live/runtime.rs` imports and keep behavior unchanged.
- [ ] Do not change tuned level constants in the same commit.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml preprocess
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live
```

---

## Task 4: Add Deterministic VAD Decisions

**Files**

- `desktop/src-tauri/src/audio/vad.rs`
- `desktop/src-tauri/src/audio/preprocess.rs`

**Interfaces**

Add a minimal endpointing shape:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VadKind {
    Speech,
    Silence,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadDecision {
    pub kind: VadKind,
    pub rms: f32,
    pub threshold: f32,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergyVadConfig {
    pub speech_rms_threshold: f32,
    pub tail_padding_ms: u32,
}

pub fn classify_energy(
    samples: &[f32],
    sample_rate_hz: u32,
    start_ms: u64,
    config: EnergyVadConfig,
) -> VadDecision;
```

**Steps**

- [ ] Implement an energy-based VAD helper for deterministic tests and initial manifests.
- [ ] Use the current `rms_level` helper.
- [ ] Mark invalid sample rates or impossible windows as `error`.
- [ ] Do not delete audio when VAD returns `silence` or `error`.
- [ ] Do not wire the energy VAD into live token gating in this plan.
- [ ] Add tests for speech, silence, and invalid sample-rate behavior.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml vad
```

---

## Task 5: Build Chunk And Session Manifests Without Server Transport

**Files**

- `desktop/src-tauri/src/audio/manifest.rs`
- `desktop/src-tauri/src/live/recordings.rs`
- `desktop/src-tauri/src/live/runtime.rs`

**Interfaces**

Add a manifest builder that works on processed frames:

```rust
pub struct AudioChunkEnvelopeBuilder {
    session_id: u64,
    sequence_start: Option<u64>,
    purpose: AudioPurpose,
    codec: AudioCodec,
    frames: Vec<AudioFrame>,
}

impl AudioChunkEnvelopeBuilder {
    pub fn new(session_id: u64, purpose: AudioPurpose, codec: AudioCodec) -> Self;
    pub fn push(&mut self, frame: AudioFrame);
    pub fn finish(self, vad_segments: Vec<VadSegment>) -> Option<AudioChunkEnvelope>;
}

pub struct AudioSessionEnvelopeBuilder {
    session_id: u64,
    source: AudioSource,
    started_at_ms: u64,
    sample_rate_hz: u32,
    chunks: Vec<AudioChunkEnvelope>,
    degraded: bool,
}

impl AudioSessionEnvelopeBuilder {
    pub fn new(session_id: u64, source: AudioSource, started_at_ms: u64, sample_rate_hz: u32) -> Self;
    pub fn push_chunk(&mut self, chunk: AudioChunkEnvelope);
    pub fn mark_degraded(&mut self);
    pub fn finish(self) -> AudioSessionEnvelope;
}
```

**Steps**

- [ ] Implement the builder with deterministic sequence ordering.
- [ ] Return `None` for an empty builder.
- [ ] Keep payload codec as `PcmS16Le` for now.
- [ ] Do not add an `Opus` enum value until the server WSS/upload contract needs it.
- [ ] Add a session manifest builder that records chunks, source, sample rate, and degraded state.
- [ ] Preserve architecture-required `vad_segments` as a vector, even when the first implementation only has one segment.
- [ ] Add tests for contiguous frames, empty builders, session manifests, and retry/idempotency fields.
- [ ] Do not change the `live-session-saved` event payload in this plan.
- [ ] Do not add manifest fields to `SavedLiveSession` in this plan.
- [ ] If live manifests are persisted during implementation, write a sidecar `live-*.manifest.json` next to the existing WAV/TXT files without changing history ingestion.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml manifest
```

---

## Task 6: Add Windowing And Tail Rules For Future Server Chunks

**Files**

- `desktop/src-tauri/src/audio/manifest.rs`
- `desktop/src-tauri/src/audio/vad.rs`
- `desktop/src-tauri/src/audio/frame.rs`

**Interfaces**

Add a pure chunking helper that does not upload:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkWindowConfig {
    pub target_window_ms: u32,
    pub max_window_ms: u32,
    pub tail_padding_ms: u32,
    pub preserve_silence_markers: bool,
}

pub fn build_manifest_windows(
    session_id: u64,
    frames: &[AudioFrame],
    vad: &[VadDecision],
    purpose: AudioPurpose,
    codec: AudioCodec,
    config: ChunkWindowConfig,
) -> Vec<AudioChunkEnvelope>;
```

**Steps**

- [ ] Use stable session-relative windows rather than wall-clock timestamps.
- [ ] Treat energy VAD decisions as deterministic test/fallback metadata only.
- [ ] Official upload/live `vad_segments` must come from Silero-derived VAD or a later Phase 3/6 server contract field.
- [ ] Return an empty vector for empty frames.
- [ ] Reject or mark `error` for mixed session ids or mixed sample rates; do not silently merge them.
- [ ] Define behavior for mismatched VAD lengths: use VAD decisions whose time ranges overlap frames, and use unsegmented `error` chunks for uncovered ranges.
- [ ] Allow VAD speech boundaries to close a chunk before `max_window_ms`.
- [ ] Preserve tail padding by extending the final speech chunk up to `tail_padding_ms` when samples exist.
- [ ] Treat `EnergyVadConfig.tail_padding_ms` as a classifier hint and `ChunkWindowConfig.tail_padding_ms` as the manifest-window extension. Do not apply tail padding twice.
- [ ] When `preserve_silence_markers` is true, emit silence manifests without dropping source audio.
- [ ] If VAD is missing or returns `error`, return one unsegmented chunk per target window and mark the VAD decision as `error`.
- [ ] Add tests for target windows, VAD boundary closure, final-word tail padding, silence marker preservation, and VAD-error fallback.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml manifest_windows
```

---

## Task 7: Wire Live Runtime To The Shared Helpers

**Files**

- `desktop/src-tauri/src/live/runtime.rs`
- `desktop/src-tauri/src/live/stream.rs`
- `desktop/src-tauri/src/live/recordings.rs`
- `desktop/src-tauri/src/audio/preprocess.rs`

**Steps**

- [ ] Replace direct calls to local preprocessing functions with `crate::audio::preprocess::*`.
- [ ] Keep `stream::chunk_samples()` in `live/stream.rs`; it is ASR-model-specific.
- [ ] Keep capture loop ownership and session stop/save logic in `live/runtime.rs`.
- [ ] Add a processed-frame timeline in the audio worker only if manifests are persisted in this pass: track `sequence`, `start_ms`, `duration_ms`, `sample_count`, and sample rate after resampling.
- [ ] If manifests stay pure/test-only in this pass, do not add frame accumulation to live runtime.
- [ ] Ensure saved live WAV output remains byte-for-byte compatible for the same input samples where practical.
- [ ] Ensure the `live-session-saved` event payload does not change unless manifest metadata is deliberately appended.
- [ ] Add one Rust test that exercises downmix, resample, and PCM conversion through the new module path.
- [ ] Leave the private `resolve_capture_device` path untouched unless this task explicitly consolidates it with `live/devices.rs`.
- [ ] If device selection is consolidated, add tests for default fallback and denied/missing device behavior against the shared selector.
- [ ] Preserve existing no-server behavior: larger recordings remain blocked or queued for server instead of running local batch fallback.
- [ ] Preserve the existing bounded/drop behavior in live runtime: bounded raw channel, single-slot readiness gate, `try_send` drop behavior, and bounded stream channel.
- [ ] Do not add long-lived sample vectors that duplicate the existing in-memory live WAV PCM buffer.

**Verification**

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml live
cd .\desktop
pnpm test -- app-types
```

---

## Task 8: Document The Local Boundary In Code And Tests

**Files**

- `desktop/README.md`
- `docs/specs/local-audio-preprocessing-stack.md`
- `docs/specs/client-state-machine.md`
- `desktop/src-tauri/src/audio/mod.rs`

**Steps**

- [ ] Add a short `desktop/README.md` note that `audio/` is deterministic preprocessing, not ASR routing.
- [ ] Do not change the proposed module names or boundaries during implementation. If a boundary needs to move, stop and update the spec first.
- [ ] Update `docs/specs/client-state-machine.md` only if new manifest state affects recording-job statuses.
- [ ] Add a module-level comment in `audio/mod.rs` stating the client/server split:

```rust
//! Deterministic desktop-side audio preparation.
//! Heavy inference, diarization, enrichment, and team storage stay server-owned.
```

- [ ] Do not add a server README section for unfinished upload code.

**Verification**

```powershell
rg -n "Whisper|Parakeet|Moonshine|GPU selector|frontend/src-tauri" desktop/src-tauri/src/audio docs/specs/local-audio-preprocessing-stack.md desktop/README.md
```

The command may match negative guardrail language in docs. It must not match implementation code that adds those paths.

---

## Task 9: Final Verification And Commit

**Files**

- All files touched by the tasks above.

**Steps**

- [ ] Format Rust:

```powershell
cargo fmt --manifest-path .\desktop\src-tauri\Cargo.toml
```

- [ ] Run Rust tests:

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
```

- [ ] Run frontend tests and build to catch type/path drift caused by moved files:

```powershell
cd .\desktop
pnpm test
pnpm build
cd ..
```

- [ ] Confirm no server connector or package churn slipped in:

```powershell
git diff -- server desktop/package.json desktop/pnpm-lock.yaml desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock
```

- [ ] Confirm no local batch fallback or server transport was added:

```powershell
rg -n "serverLive|serverBatch|queued_local_fallback|blocked_server_unavailable|upload|websocket|ws://" desktop/src desktop/src-tauri/src
```

Review expected matches in existing state-machine and live-route code. There must be no new local batch transcription path and no unfinished server transport.

- [ ] Commit only the audio preprocessing changes:

```powershell
git status --short
git add desktop/src-tauri/src/audio desktop/src-tauri/src/live/runtime.rs desktop/src-tauri/src/live/stream.rs desktop/src-tauri/src/live/recordings.rs desktop/README.md docs/specs/client-state-machine.md
git commit -m "refactor: extract local audio preprocessing"
```

**Expected Outcome**

- Local live fallback behavior stays intact.
- Reusable audio helpers no longer live inside the live runtime.
- Future server upload specs can map from deterministic local capture-envelope types without the desktop predefining the server contract.
- The desktop remains a thin client with local live fallback, not a local meeting-backend clone.
