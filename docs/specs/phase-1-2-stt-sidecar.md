# Spec: Local Moonshine Fallback Sidecar

**Status:** Scoped to PR3.

## Scope

- One local CrispASR sidecar.
- One local model family: Moonshine tiny for live/offline degraded use.
- Pinned and verified artifacts: CrispASR binary, Moonshine GGUF, tokenizer, FireRed punctuation.
- Loopback HTTP requires a per-launch API key.
- Rust writes transcript files beside source files.

## Not In Scope

- Python runtime fallback.
- Local Cohere as default batch runtime.
- Runtime backend switching.
- CLI-per-file fallback.
- DGX/server batch connector.

## Product Rule

Live/offline fallback uses local Moonshine. Larger recordings should use the DGX/server Cohere path when available; if offline without a suitable cached GPU/server path, queue or block instead of running a low-quality laptop fallback.

## Checks

- `cargo test --locked`
- `npm test`
- `npm run build`
- Sidecar smoke: unauthenticated request returns `401`; bearer-authenticated request returns `200`.
