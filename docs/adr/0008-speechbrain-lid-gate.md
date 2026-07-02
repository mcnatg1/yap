# ADR 0008: SpeechBrain LID language gate

**Date:** 2026-06-30
**Status:** Accepted (roadmap — Phase 4)
**Builds on:** [ADR 0003](0003-long-term-voice-architecture.md) (resolves its open questions), [ADR 0002](0002-crispasr-unified-stt-runtime.md) (Cohere needs explicit `-l`)

## Context

Cohere requires an explicit language code and does **not** auto-detect; a wrong `-l` produces badly garbled output. ADR 0003 established the **gate pattern** (suggest, never silently switch) but left four items open: checkpoint, probe duration, bundle-vs-download, and disagreement handling. This ADR closes them for **batch only** (Phase 4); live stays English-only.

## Decision

### Model

| Item | Decision |
|------|----------|
| Checkpoint | `speechbrain/lang-id-voxlingua107-ecapa` (ECAPA-TDNN, 107 langs) |
| Footprint | ~50 MB weights; runtime peak ~300 MB; CPU only |
| Process | Lightweight Python micro-service / subprocess, **batch probe only** — not on any hot path |
| Delivery | **On-demand download** on first "Detect language" use (keep installer lean); cache in `YAP_MODELS_DIR` |

### Probe strategy (adaptive, max 2 windows)

```
1. Extract 15 s probe from file start → LID → { iso, conf }
2. If conf ≥ 0.70 → use it
3. Else extract a 2nd 15 s window from file middle → LID
4. Agree (same mapped code) → use it (raised confidence)
   Disagree or still < 0.70 → LOW-CONFIDENCE gate (manual pick)
```

Cap at **2 windows** (~30 s audio) to bound cost. Cache result **per file**.

### Code mapping

- Map VoxLingua107 ISO codes → Cohere's 14 (`en fr de it es pt el nl pl zh ja ko vi ar`).
- Maintain explicit **unsupported** list and **ambiguous** collapses (e.g. `zh-*` → `zh`).
- Unsupported detected → "not supported yet" gate with closest-supported option.

### Gate UX (from ADR 0003, normative)

| LID outcome | Behavior |
|-------------|----------|
| High conf + supported | Prefill; toast "Detected French — transcribe in French?" [Continue]/[Pick another] |
| Low conf / disagree | "Couldn't detect confidently." [Choose language] |
| Unsupported | "Not supported yet (detected …)." [Choose closest]/[Cancel] |

Manual picker is **always available**; LID is assistive, never mandatory.

### Language memory
Remember last confirmed `-l` per source folder; pre-select it next time to cut repeat gates (still shows the gate, just pre-filled).

## Consequences

### Positive
- Closes ADR 0003 open questions; Phase 4 is buildable.
- Adaptive probe balances cost vs accuracy; bounded at 30 s.
- On-demand download keeps base installer small.

### Negative
- Third ML stack at steady state (CrispASR + llama-server + SpeechBrain) — ops/disk cost; isolated to a subprocess used only on demand.
- First detect has a download wait (mitigated: progress UI, optional pre-cache in Settings).

### Neutral
- Live LID deferred to the future multilingual-live ADR; this is batch-only.

## Alternatives considered

- **Silent auto `-l`** — rejected (ADR 0003): wrong-language Cohere is severe; low trust.
- **CrispASR `-l auto` / native LID** — deferred: evaluate before permanently keeping the Python SpeechBrain dependency (ADR 0003 "room for improvement #5").
- **Whisper detect-language** — rejected: extra STT stack, cloud-adjacent expectations.
- **Bundle in installer** — rejected as default: inflates download for a feature many batch users skip.

## References
- [ADR 0003](0003-long-term-voice-architecture.md) — gate pattern, language policy
- SpeechBrain: https://speechbrain.github.io/
- [Testing strategy](../specs/testing-strategy.md) — `multi-fr-30s.wav`
