# Spec: Testing strategy

**Status:** Draft (2026-06-30)
**Scope:** Cross-cutting test approach for the sidecar/worker architecture — fixtures, accuracy spot-checks, and the CI matrix that pins native runtimes.

This is the shared reference the phase specs point to for their acceptance tests.

---

## 1. Test layers

| Layer | What | Tooling |
|-------|------|---------|
| **Unit** | Pure logic — error mapping, language code map, manifest serde, path naming | `vitest` (TS), `cargo test` (Rust) |
| **Integration** | Rust ↔ sidecar over real IPC; one fixture in → expected shape out | `cargo test` w/ sidecar launched; tagged `#[ignore]` unless binaries present |
| **E2E (smoke)** | App boots, transcribes a fixture, shows transcript | Playwright/WebDriver on the Tauri build |
| **Accuracy** | WER spot-check vs golden transcripts | Python `jiwer` script in CI, tolerance-gated |

Keep unit/integration fast and offline. Accuracy + E2E run on the per-OS matrix.

---

## 2. Fixtures

Stored under `tests/fixtures/` (small, license-clear audio):

| File | Purpose | Expectation |
|------|---------|-------------|
| `en-60s.wav` | English batch + live | WER ≤ target vs `en-60s.golden.txt` |
| `multi-fr-30s.wav` | `-l fr` batch | Non-empty French; LID detects `fr` |
| `silence-5s.wav` | VAD/no-speech | No phrases finalized |
| `corrupt.m4a` | decode failure | `AUDIO_DECODE` error |
| `two-speaker-2min.wav` | Phase 7 diarization | ≥2 `SPEAKER_XX`, stable across chunks |

Golden transcripts live beside fixtures. Comparison is **WER-tolerant**, never byte-equal (quantized models drift).

---

## 3. Accuracy spot-checks (WER)

- Script: `tests/wer_check.py` → `jiwer` WER between fixture output and golden.
- Gates (tune with real data; starting points):

| Path | WER gate |
|------|----------|
| Cohere Q4 batch (en) | ≤ 0.12 |
| Cohere Q8 batch (en) | ≤ 0.10 |
| Moonshine live (en, finals) | ≤ 0.18 |

- A regression beyond gate **fails CI** for that backend; quant toggle (Q8) is the documented mitigation.

---

## 4. CI matrix (pinned native runtimes)

The risk is **native binaries**, not app logic. CI must run the pinned `crispasr` and `llama-server` per OS.

| OS | crispasr smoke | llama-server smoke | E2E |
|----|----------------|--------------------|-----|
| Windows x64 | ✅ load + 1 file | ✅ 1 completion | ✅ |
| macOS arm64 | ✅ | ✅ | ✅ |
| macOS x64 | if retained | if retained | — |
| Linux x64 | best-effort | best-effort | — |

- Versions pinned in `desktop/crispasr-version.txt` and `desktop/llama-model.txt` (+ llama.cpp build hash).
- Smoke = start sidecar at pinned version, run one fixture, assert non-empty output + clean exit. Catches breaking changes on upgrade ([ADR 0002](../adr/0002-crispasr-unified-stt-runtime.md) requirement).

---

## 5. Per-phase test focus

| Phase | Critical tests |
|-------|----------------|
| 1–2 STT | sidecar parity, crash→restart→complete, port-conflict, `python` fallback, queue continues past corrupt file |
| A–D LLM | Polish parity, 400 ms Scribe bypass, backend flag, empty-completion retry |
| 3 Live | partial latency, silence finalize, raw-mode badge, mic-denied recovery, dual-STT block |
| 4 LID | code mapping, low-confidence gate, multi-window probe agreement |
| 7a–c L3 | speaker-vault stability across chunks, align-on-raw, FIFO degraded mode, quarantine on bad write |
| 7d–e Agents | citation-required Analyst, three-strike Student, RAG confidence floor |

---

## 6. Non-goals

- No cloud test infra (local-first; fixtures are committed/small).
- No load/perf benchmarking suite in v1 beyond the WER + latency spot-checks above.
- No telemetry — debugging uses local logs (`%LOCALAPPDATA%/Yap/logs/`).
