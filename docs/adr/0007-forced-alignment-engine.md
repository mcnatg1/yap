# ADR 0007: Forced-alignment engine for word→speaker

**Date:** 2026-06-30
**Status:** Accepted alignment principle (canonical Phase 6); exact engine requires benchmark revalidation
**Builds on:** [ADR 0004](0004-background-diarization-okf-agents.md) (resolves the aligner "TBD"), [ADR 0002](0002-crispasr-unified-stt-runtime.md) (CrispASR runtime)
**Amended by:** [ADR 0020](0020-meeting-capture-diarization-authority.md) - alignment consumes revisioned source-aware diarization results. Its resource gate is measured with the selected diarization path rather than a fixed WeSpeaker companion.

> **Applicability:** Align raw STT and preserve word timestamps. The historical Canary/Wav2Vec2 engine selection is a benchmark candidate, not permission to add either runtime without current accuracy, licensing, CPU, memory, and packaging evidence.

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
2. **Footprint** — model + runtime peak < 300 MB within the measured shared preprocessing and diarization budget.
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
- Alignment is canonical **Phase 6**; the historical `7a` label is no longer used.

## Alternatives considered

- **WhisperX-style alignment** — rejected: pulls Whisper stack we don't otherwise use.
- **Single canary for everything (incl. en)** — rejected as default: wav2vec2-en is lighter/faster for the most common case; canary stays the multilingual default.
- **No alignment, sentence-level speaker guess** — rejected: chunk identity drift and poor attribution; overlap rule needs word timings.

## References
- [ADR 0004](0004-background-diarization-okf-agents.md) — diarization, intersection, worker
- [Testing strategy](../specs/testing-strategy.md) — source-aware meeting fixtures and timing gates
