# Spec: Local Nemotron Live Fallback

**Status:** Implemented baseline; native CI smoke, performance gates, and release packaging remain

## Scope

- One local live/offline streaming model: Nemotron 3.5 ASR Streaming 0.6B INT8.
- One runtime path: in-process `sherpa-onnx` owned by the Tauri live worker.
- Pinned and verified artifacts: encoder, decoder, joiner, and tokens file.
- Setup/settings own artifact download, removal, and fallback disablement; runtime never silently downloads.
- Rust writes live WAV/TXT files into app history after local sessions.

## Not In Scope

- Python runtime fallback.
- Client-side Cohere as the default batch runtime.
- Runtime backend/model switching.
- Client-side Parakeet or Moonshine experiments.
- GB-class server batch connector.

## Product Rule

Live/offline fallback uses local Nemotron INT8. Larger recordings should use the GB-class server path when available; if offline without a suitable server path, queue or block instead of running a laptop fallback as the official path.

## Client Workflow Rule

The desktop queue must model the real recording workflow, not a cosmetic readiness layer:

- Jobs carry setup/server/fallback routing state as typed app state.
- Jobs reserve pipeline fields for preprocessing and diarization even when those stages are not implemented yet.
- Local Nemotron INT8 is the live/offline fallback path, not the official large-recording product path.
- Larger recordings should queue or block without a server path instead of silently producing official-looking fallback transcripts.
- UI labels stay compact; docs carry the explanation.

See [client-state-machine.md](client-state-machine.md) for the typed recording-job workflow.

## Runtime Rules

- Keep the `sherpa-onnx` recognizer warm across local live sessions.
- Use 1120 ms chunks until profiling proves a smaller chunk stays real-time with acceptable WER.
- Flush buffered audio plus a short silence tail on stop so final words are not clipped.
- Log model load time, chunk count, audio duration, decode duration, and first-text latency.

## Checks

- `cargo test --locked`
- `pnpm test`
- `pnpm build`
- `cargo run --release --manifest-path .\desktop\src-tauri\Cargo.toml --example nemotron_profile -- <clip.wav> [reference.txt]`
- Live smoke: hold-to-talk and hands-free sessions save WAV/TXT entries that appear on Home.
