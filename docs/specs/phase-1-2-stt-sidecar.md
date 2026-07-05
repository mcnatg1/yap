# Spec: Local Moonshine Fallback Sidecar

**Status:** Scoped to PR3.

## Scope

- One local CrispASR sidecar.
- One local model family: Moonshine tiny for live/offline degraded use.
- Pinned and verified artifacts: CrispASR binary, Moonshine GGUF, tokenizer, FireRed punctuation.
- Setup/settings own artifact download, removal, and fallback disablement; runtime never silently downloads.
- Loopback HTTP requires a per-launch API key.
- Rust writes transcript files beside source files.

## Not In Scope

- Python runtime fallback.
- Client-side Cohere as the default batch runtime.
- Runtime backend switching.
- CLI-per-file fallback.
- GB-class server batch connector.

## Product Rule

Live/offline fallback uses local Moonshine. Larger recordings should use the GB-class server Cohere path when available; if offline without a suitable cached GPU/server path, queue or block instead of running a low-quality laptop fallback.

## Client Workflow Rule

The desktop queue must model the real recording workflow, not a cosmetic readiness layer:

- Jobs carry setup/server/fallback routing state as typed app state.
- Jobs reserve pipeline fields for preprocessing and diarization even when those stages are not implemented yet.
- Local Moonshine tiny is the live/offline fallback path, not the official large-recording product path.
- Larger recordings should queue or block without a server path instead of silently producing official-looking fallback transcripts.
- UI labels stay compact; docs carry the explanation.

See [client-state-machine.md](client-state-machine.md) for the typed recording-job workflow.

## Checks

- `cargo test --locked`
- `pnpm test`
- `pnpm build`
- Sidecar smoke: unauthenticated request returns `401`; bearer-authenticated request returns `200`.
