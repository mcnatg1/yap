# ADR 0019: Local streaming model selection

**Date:** 2026-07-08
**Status:** Accepted (canonical Phase 2 local fallback)
**Amends:** [ADR 0001](0001-dual-stt-backends.md), [ADR 0002](0002-crispasr-unified-stt-runtime.md), [ADR 0003](0003-long-term-voice-architecture.md), [ADR 0006](0006-silero-agents-state-machine.md), [ADR 0014](0014-server-tier-compute-topology.md), [ADR 0018](0018-three-repo-topology.md)

## Context

Earlier ADRs selected Moonshine v2 tiny, then Parakeet Q4, as local live fallback candidates. Both were useful probes, but local testing changed the decision:

- Moonshine was fast enough, but the accuracy floor was too low.
- Parakeet Q4 had better product intent, but the tested client runtime was not real-time enough and added CrispASR/GPU routing complexity to the desktop.
- Nemotron 3.5 ASR Streaming 0.6B INT8 through `sherpa-onnx` was the first tested local path that stayed under real-time on CPU while keeping a better transcript floor.

The client direction is now sharper:

- The desktop app is a thin client for the DGX Spark / GB-class server path when connected.
- Larger recordings stay queued or blocked locally until the server path is available.
- The only local audio inference path is live/offline streaming fallback.
- Server-side model fusion, batch routing, diarization, and heavier ASR experiments belong in `yap-server`, not the desktop client.

## Decision

Use **Nemotron 3.5 ASR Streaming 0.6B INT8** as the single pinned local streaming fallback model.

| Concern | Decision |
|---------|----------|
| Local fallback model | `nemotron-3.5-asr-streaming-0.6b-1120ms-int8` |
| Hugging Face source | `csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-1120ms-int8-2026-06-11` at pinned revision |
| Runtime | In-process `sherpa-onnx` |
| Provider | CPU for desktop fallback |
| Chunk size | 1120 ms default; lower chunks are allowed only after profiling proves they stay real-time with acceptable WER |
| Punctuation | Native model punctuation; no external punctuation companion |
| Client selector | No local model router; expose only practical compute/status controls |
| Server routing | Server owns model fusion, larger models, batch ASR, diarization, and future routing experiments |

The local pin remains fail-closed with SHA-256 verification. Runtime startup should verify cached artifacts once, write marker files, then avoid rehashing large model files on every launch.

## Benchmarks

The deciding profile used a controlled 30 s English WAV and the same streaming harness across chunk sizes. After implementation, the release-mode app profiler (`cargo run --release --example nemotron_profile`) confirmed the in-app `LiveStreamEngine` path is faster than the initial probe.

| Model/runtime | Chunk | CPU real-time factor | WER |
|---------------|-------|----------------------|-----|
| Nemotron INT8 / sherpa-onnx | 80 ms | 2.999 | 50.00% |
| Nemotron INT8 / sherpa-onnx | 160 ms | 1.561 | 29.31% |
| Nemotron INT8 / sherpa-onnx | 560 ms | 0.604 | 8.62% |
| Nemotron INT8 / sherpa-onnx probe | 1120 ms | 0.426 | 6.90% |
| Nemotron INT8 / app profiler release | 1120 ms | 0.299 | 6.90% |
| Parakeet Q4 / CrispASR 0.8.8 RTX Vulkan | 1120 ms | 1.442 | 50.00% |
| Parakeet Q4 / CrispASR 0.6.12 RTX Vulkan | 1120 ms | 3.741 | 43.10% |
| Parakeet Q4 / CrispASR CPU | 1120 ms | 3.026 | 51.72% |

Real-world sanity profile on `00018_Wireless GO.WAV`: 88.1 s audio, 23.6 s decode, RTF 0.268, first text at 577 ms.

Real-time factor below `1.0` is required for local live fallback. On the tested path, Nemotron 1120 ms is the current default because it provides the best accuracy/speed balance.

## Consequences

- Local live/offline fallback has a better accuracy floor than the Moonshine tiny path and actually stays under real-time on CPU.
- The desktop app avoids client-side model routing bloat.
- GPU routing, Parakeet experiments, and any fusion strategy move to the server.
- The setup flow downloads one pinned Nemotron artifact set instead of CrispASR plus GGUF/tokenizer/punctuation companions.
- Streaming latency work now focuses on one runtime path: sherpa chunking, VAD boundaries, warm recognizer behavior, and finalization timing.
- Historical Moonshine and Parakeet wording in earlier ADRs remains prior context unless explicitly amended by this ADR.

## Implementation Notes

- `desktop/src-tauri/src/stt/nemotron.rs` pins and verifies the Nemotron artifact set.
- `desktop/src-tauri/src/live/stream.rs` owns the in-process `sherpa-onnx` recognizer.
- `desktop/src-tauri/src/live/runtime.rs` keeps the recognizer warm across stop/start boundaries and logs chunk/audio/decode timing.
- Batch or long-form recording work should not add a second desktop-local ASR model; it should queue for the server API.

## Alternatives Considered

### Keep Moonshine v2 tiny

Rejected as the default. It is small and fast, but the accuracy floor was too low for the intended local live fallback experience.

### Keep Parakeet Q4 locally

Rejected for the desktop client. It may still be useful server-side, but the tested client runtime was slower than real-time and carried too much local routing/GPU complexity.

### Add a desktop model router

Rejected for MVP. Routing between Moonshine, Parakeet, Nemotron, and future models would increase state-machine and setup complexity on the client. Server routing is the right home for that experimentation.

### Remove local ASR entirely

Rejected for now. The product still needs an offline/degraded live dictation path when the server is unavailable.
