# Local Nemotron Live Transcription Implementation Plan

> **For agentic workers:** Use this as the current implementation map. ADR 0019 supersedes earlier Moonshine/Parakeet local fallback plans.

**Goal:** Keep the desktop client focused on one local live/offline fallback: Nemotron 3.5 ASR Streaming 0.6B INT8 via `sherpa-onnx`.

**Spec:** [../specs/2026-07-05-local-nemotron-live-transcription.md](../specs/2026-07-05-local-nemotron-live-transcription.md)

**Architecture:** React stays a view layer. Tauri Rust owns model setup, mic capture, resampling, the warm recognizer, live-session state, and local WAV/TXT save. The client does not keep a local Parakeet/CrispASR child or GPU routing path.

## Files

- `desktop/src-tauri/src/stt/nemotron.rs`: pinned model artifact set, checksums, install/remove/status.
- `desktop/src-tauri/src/stt/model.rs`: shared model-directory, download, and checksum helpers.
- `desktop/src-tauri/src/live/stream.rs`: `sherpa-onnx` recognizer config and stream engine.
- `desktop/src-tauri/src/live/runtime.rs`: CPAL capture, session tokens, worker lifecycle, local save.
- `desktop/src-tauri/src/live/state.rs`: serializable live session projection.
- `desktop/src-tauri/src/lib.rs`: Tauri commands and state ownership.
- `desktop/src/live.ts`: TypeScript command/event boundary.
- `desktop/src/components/live/*`: overlay projection and interaction UI.
- `desktop/src/App.tsx`: setup state and history hydration.
- `docs/adr/0019-local-streaming-model-selection.md`: model decision and benchmark record.

## Constraints

- Do not add a local model router.
- Do not reintroduce local Cohere, Parakeet, Moonshine, or CrispASR child processes.
- Do not run official large-recording transcription locally when the server is unavailable.
- Do not perform model inference or filesystem work inside the CPAL callback.
- Do not erase final text on stop/crash before the save path has completed.
- Keep punctuation enabled through the native model/runtime behavior.

## Remaining Work

- [ ] Add a committed or mocked audio fixture path so CI cannot silently skip runtime parity.
- [ ] Add a focused Tauri command/capability hardening pass for model install/remove/open/reveal paths.
- [ ] Add Rust Silero ONNX and emitted `vad_segments` after live capture is stable.
- [ ] Add Opus/WSS server connector after the DGX/server API contract exists.
- [ ] Keep Playwright/WebDriver coverage on overlay state transitions so motion regressions are visible.

## Verification

```powershell
cargo test --locked --manifest-path .\desktop\src-tauri\Cargo.toml
cargo clippy --locked --manifest-path .\desktop\src-tauri\Cargo.toml --all-targets -- -D warnings
pnpm test
pnpm build
cargo run --release --manifest-path .\desktop\src-tauri\Cargo.toml --example nemotron_profile -- <clip.wav> [reference.txt]
```
