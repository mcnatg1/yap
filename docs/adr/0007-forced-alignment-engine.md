# ADR 0007: Forced-alignment engine for word→speaker

**Date:** 2026-06-30
**Status:** Accepted (roadmap — Phase 7a)
**Builds on:** [ADR 0004](0004-background-diarization-okf-agents.md) (resolves the aligner "TBD"), [ADR 0002](0002-crispasr-unified-stt-runtime.md) (CrispASR runtime)

## Context

L3 needs **word-level timestamps** to intersect transcript words with diarization segments ([ADR 0004 §5](0004-background-diarization-okf-agents.md)). Cohere and Moonshine emit **plain text without reliable word timings**, so a forced aligner is mandatory. ADR 0004 named `canary-ctc-aligner` as default but left the final pick "TBD".

Constraints: runs **inside the knowledge worker** (subprocess, ≤2 ORT threads, <300 MB), must align to **raw STT text** (never polished), and should cover the 14 batch languages — not just English.

## Decision

**Two-tier aligner, selected per chunk language:**

| Tier | Engine | Used for | Runtime |
|------|--------|----------|---------|
| **Default (multilingual)** | `canary-ctc-aligner` GGUF | any of the 14 batch langs | Invoked via CrispASR CLI `-am` **when the pinned sidecar exposes it**, else ONNX in worker |
| **Fast path (English)** | `wav2vec2`-en CTC ONNX | `language == en` (live + en batch) | ONNX in worker (`ort`, 2 threads) |

Selection rule in worker: `en → wav2vec2-en`; otherwise `canary`. Both produce `[{ word, t0_ms, t1_ms }]`; the **Majority Overlap Rule** (>50%) from ADR 0004 §5 is unchanged.

**Always align the `text_raw` field** from the chunk manifest; polished text inherits labels by shared word index/timestamps.

## Decision criteria (final pick at build)

Confirm the default against fixtures before locking:

1. **Coverage** — canary must align all 14 codes; if a language fails, fall back to whole-chunk speaker (single dominant speaker) rather than crash.
2. **Footprint** — model + runtime peak < 300 MB alongside WeSpeaker.
3. **Quality** — word boundary error acceptable for >50% overlap rule (coarse is fine; we need segment attribution, not karaoke).
4. **Invocation** — prefer CrispASR-exposed aligner (one runtime) over a second ONNX dependency; choose worker-ONNX only if the pinned CrispASR build lacks `-am`.

## Consequences

### Positive
- English live/batch gets a small, fast, proven aligner.
- Multilingual recordings still get speaker attribution.
- Reuses CrispASR when possible — fewer runtimes to ship.

### Negative
- Two alignment code paths to test (mitigated: same output schema, shared intersection).
- Canary footprint/quality unverified until benched on real media.

### Neutral
- Alignment is **Phase 7a**; nothing ships until L3 does.

## Alternatives considered

- **WhisperX-style alignment** — rejected: pulls Whisper stack we don't otherwise use.
- **Single canary for everything (incl. en)** — rejected as default: wav2vec2-en is lighter/faster for the most common case; canary stays the multilingual default.
- **No alignment, sentence-level speaker guess** — rejected: chunk identity drift and poor attribution; overlap rule needs word timings.

## References
- [ADR 0004](0004-background-diarization-okf-agents.md) — diarization, intersection, worker
- [Testing strategy](../specs/testing-strategy.md) — `two-speaker-2min.wav` fixture
