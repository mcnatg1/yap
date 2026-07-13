# Spec: Testing strategy

**Status:** Living verification contract (updated 2026-07-12); future phase gates activate only when their fixtures exist
**Scope:** Cross-cutting tests for the desktop runtime, track-aware audio contracts, local fallback, source-aware diarization, server contracts, and native UI.

This is the shared reference the phase specs point to for their acceptance tests.

**Current activation:** deterministic generated-tone and contract fixtures exist. The licensed speech clips/golden transcripts, `tests/wer_check.py`, meeting RTTM manifest, diarization benchmark harness, bundled llama-server, and per-OS real-model matrix described below do not exist yet. Their tables are target gates, not claims about active CI.

---

## 1. Test layers

| Layer | What | Tooling |
|-------|------|---------|
| **Unit** | Pure logic — error mapping, language code map, manifest serde, path naming | `vitest` (TS), `cargo test` (Rust) |
| **Integration** | Rust ↔ sidecar over real IPC; one fixture in → expected shape out | `cargo test` w/ sidecar launched; tagged `#[ignore]` unless binaries present |
| **E2E (smoke)** | App boots, overlay responds, desktop shell opens | Playwright for browser/Tauri shell surfaces; WebdriverIO for true desktop smoke |
| **Accuracy** | WER spot-check vs golden transcripts | Python `jiwer` script in CI, tolerance-gated |
| **Diarization** | DER/JER, speaker count, short-turn recall, overlap, and identity false-name gates | License-clear RTTM fixtures + benchmark harness |
| **Reliability/privacy** | Gap recovery, reconnect revisions, consent, deletion, tenant isolation | Rust integration and server contract tests |
| **Performance** | Capture drops, ASR regression, CPU, RSS, and RTF | Deterministic profiler jobs; hardware results recorded separately from CI pass/fail when hosts differ |

Keep unit/integration fast and offline. Accuracy + E2E run on the per-OS matrix.

### Windows installer safety boundary

Yap uses Tauri's stock NSIS template without installer hooks. The application and Tauri agree on the
same canonical data namespace: Tauri `app_data_dir()` for `com.mcnatg1.yap`, which is
`%APPDATA%\com.mcnatg1.yap` on Windows. Local development may build the bundle and run static
release contracts, but must not execute the production installer lifecycle on an everyday profile.

The lifecycle smoke runs only on a fresh GitHub-hosted Windows runner or in an explicitly disposable
Windows VM (`YAP_DISPOSABLE_WINDOWS=1`). It fails if production install, registry, or app-data state
already exists; verifies the installer hash when supplied; uses bounded process handles for install,
app launch, and uninstall; proves the app writes its log under the canonical Tauri directory; and
uses stock silent uninstall. Stock silent uninstall preserves app data. The workflow makes no
automated delete-data claim and performs no script-owned recursive cleanup; disposal of the Windows
environment is the cleanup boundary.

The supported release workflow stages a verified GitHub draft from an immutable commit on `main`.
The private repository plan does not support required-reviewer environment rules, so the workflow
never publishes the draft itself. Final publication is an explicit GitHub UI action after reviewing
the draft assets and `release-metadata.json`.

### Overlay and motion contract

The live overlay has two test owners:

| Owner | Covers | Must catch |
|-------|--------|------------|
| Playwright preview mode | DOM layout, visible island dimensions, hover/recording/processing/success states, reduced-motion behavior | One-frame layout jumps, hit-area shrink, overlap, text overflow |
| WebdriverIO desktop smoke | Real Tauri overlay window properties and tray/app-window behavior | Taskbar/Alt-Tab exposure, focusability, native frame size drift |

For overlay changes, settled screenshots are not enough. Add a short `requestAnimationFrame` sampler around hover/state churn and fail on unexpected rect drift. During native resize, the top hover target must remain reachable through the collapse grace period while window bounds continue to match the visible island.

---

## 2. Fixtures

Current generated fixtures:

| Path | Purpose | Expectation |
|------|---------|-------------|
| `desktop/tests/fixtures/audio-fixture.ts` | Deterministic 16 kHz mono WAV generator for UI/contract tests | Stable bytes; not treated as speech quality evidence |

Future speech fixtures should be stored under `desktop/tests/fixtures/`
(small, license-clear audio):

| File | Purpose | Expectation |
|------|---------|-------------|
| `en-60s.wav` | English batch + live | WER ≤ target vs `en-60s.golden.txt` |
| `multi-fr-30s.wav` | `-l fr` batch | Non-empty French; LID detects `fr` |
| `silence-5s.wav` | VAD/no-speech | No phrases finalized |
| `corrupt.m4a` | decode failure | `AUDIO_DECODE` error |
| `meeting-one-speaker.wav` + RTTM | Baseline attribution | One stable anonymous cluster; no false name |
| `meeting-two-speaker.wav` + RTTM | Turn-taking diarization | DER/JER and speaker-count gates |
| `meeting-short-turns.wav` + RTTM | Sub-1.6 s evidence | Short turns preserved; weak evidence may remain unknown |
| `meeting-overlap.wav` + RTTM | Concurrent speakers | Overlap scored explicitly; challenger promotion gate |
| `meeting-echo-two-track/` | Future mic/system leakage | No duplicate speaker inflation; track drift and gaps represented |

Golden transcripts live beside fixtures. Comparison is **WER-tolerant**, never byte-equal (quantized models drift).

Real sidecar parity tests stay opt-in: set `YAP_PARITY_CLIP` and run the ignored
Cargo parity tests when a licensed audio clip is available. Normal CI uses
`desktop/src-tauri/tests/fixtures/parity-contract.verbose.json` to keep
timestamp-shape coverage without shipping private or unclear audio.

---

## 3. Accuracy spot-checks (WER)

- Script: `tests/wer_check.py` → `jiwer` WER between fixture output and golden.
- Gates (tune with real data; starting points):

| Path | WER gate |
|------|----------|
| Server Cohere batch (en) | ≤ 0.12 |
| Nemotron INT8 live (en, finals) | ≤ 0.18 |

- A regression beyond gate **fails CI** for that backend; server pool sizing/model choice is the mitigation.

### Diarization and identity gates

Starting targets from the source-aware design:

| Metric | Gate |
|--------|------|
| No-collar DER with overlap scored | ≤ 0.20 |
| Speaker-count mean absolute error | ≤ 0.5 |
| Named identity precision | ≥ 0.995 |
| Open-set false-name rate | ≤ 0.001 |
| Local anonymous diarization RSS increase | < 150 MB |
| Client p95 CPU increase on reference hardware | < 5 percentage points |
| Local-ASR latency regression while evidence is active | < 10% |
| Supported-load audio callback drops | 0 |

Named-identity gates remain inactive until the purpose-authorized server identity phase exists. Anonymous clustering must never manufacture a name to improve a metric.

The approved diarization suite is a checked-in `desktop/tests/fixtures/diarization/manifest.json` plus license/provenance records, SHA-256 hashes, audio, transcripts, and RTTM annotations for the meeting cases above. The baseline cannot be accepted while any required fixture or license record is missing. The initial client reference profile is Windows 11 x64, CPU-only, 4 physical cores/8 threads in the Intel Core i5-1135G7 performance class, 16 GB RAM, normal process priority, and the OS balanced power plan. Every benchmark result records exact CPU, RAM, OS build, runtime/model revisions, and power mode.

The supported-load callback test runs 48 kHz stereo capture converted to the required prepared format while local ASR, recording, and anonymous speaker evidence are active. It includes deterministic queue saturation and a four-hour accelerated timeline. Hardware-specific performance gates run on the pinned reference host; portable CI still runs deterministic contract, fixture-shape, and loss-accounting tests.

---

## 4. CI matrix (pinned native runtimes)

The risk is **native runtimes**, not app logic. CI must run the pinned Nemotron/sherpa path and `llama-server` per OS.

| OS | Nemotron live smoke | llama-server smoke | E2E |
|----|----------------|--------------------|-----|
| Windows x64 | ✅ profiler + fixture | ✅ 1 completion | ✅ |
| macOS arm64 | ✅ | ✅ | ✅ |
| macOS x64 | if retained | if retained | — |
| Linux x64 | best-effort | best-effort | — |

- Versions pinned in `desktop/src-tauri/src/stt/nemotron.rs` and `desktop/llama-model.txt` (+ llama.cpp build hash).
- Smoke = run `nemotron_profile` against one fixture, assert non-empty output + real-time factor under gate. Catches breaking changes on upgrade ([ADR 0019](../adr/0019-local-streaming-model-selection.md)).

---

## 5. Per-phase test focus

| Phase | Critical tests |
|-------|----------------|
| 1–2 STT | Nemotron profiler, live state transitions, local fallback disabled, queue blocks larger files without server |
| A–D LLM | Polish parity, 400 ms Scribe bypass, backend flag, empty-completion retry |
| 3 Live | partial latency, silence finalize, raw-mode badge, mic-denied recovery, dual-STT block |
| 4 LID | code mapping, low-confidence gate, multi-window probe agreement |
| 6 Preprocessing | mixed-session rejection, track-aware content IDs, explicit gaps, bounded windows, advisory VAD |
| 7 Identity/access | Yap API token audience, `(tid, oid)` isolation, consent and withdrawal, profile-version compatibility |
| 8 Meeting evidence | one/two/overlap/short/noisy speakers, stable result revisions, bounded clusters, no local names or persistent embeddings |
| 7d–e Agents | citation-required Analyst, three-strike Student, RAG confidence floor |

---

## 6. Client state machine tests

- Rust transition-table tests cover runtime invariants: live vs batch exclusion, large-recording block when server is offline, fallback setup races, and finish/error transitions.
- Frontend projection tests cover setup/server labels, blocked jobs, retry rows, and history-to-job conversion.
- Future contract tests cover server health/auth, batch upload/job status, live WSS tokens, and fallback events.
- Event-order tests must use job IDs before server upload work ships.

## 7. Source-aware meeting tests

- `SessionMode`, trigger gesture, physical `CaptureSource`, local speaker slot, session speaker, and durable identity are independently serialized and validated.
- Recording remains correct when ASR, speaker evidence, or transport is absent, backpressured, or crashed.
- Long meeting recording uses bounded memory; an interrupted write is recoverable and cannot appear complete.
- Cross-session and cross-track frames fail closed instead of being relabeled.
- Lost callback intervals produce explicit gaps and a partial/degraded result.
- A saturated callback handoff reports the exact loss through the reserved accumulator.
- A callback update racing an accumulator drain appears in the next loss generation.
- `Unknown` may pass through a hidden candidate state and become `Speaker N` in a new revision; neither state may become a local name.
- Repeated weak evidence may establish an anonymous cluster but cannot update a profile.
- Speaker-turn and aligned-word intervals are end-exclusive, monotonic, bounded by the capture timeline, and preserve overlap.
- Alignment failure leaves timestamped speaker turns intact and omits or marks word timing unavailable.
- The local baseline passes the absolute DER, speaker-count, CPU, RSS, latency, and callback-drop gates before release.
- Server reconciliation appends a revision and cannot silently overwrite a user correction.
- Contact import and transcript renaming create no biometric enrollment.
- Unenrolled, withdrawn, expired, cross-tenant, and incompatible-model profiles cannot match; enrollment, matching, and adaptation grants are checked separately, and matching-grant withdrawal denies naming without requiring profile deletion.
- Same replay key/same hash is idempotent; same key/different hash conflicts; different keys/same hash remain distinct.
- Fault injection around every recording commit step cannot produce a false-complete session.
- Withdrawal during in-flight matching prevents publication, and backup restore honors deletion tombstones.
- Replayed server results apply an authorized profile adaptation at most once; conflicting evidence fails closed.
- Transient embeddings are absent from logs, sidecars, temporary artifacts, and SQLite after normal and crashed runs.
- Four-hour and 64-speaker synthetic tests prove bounded memory and assignment state.

---

## 8. Non-goals

- No cloud test infra (local-first; fixtures are committed/small).
- No generic enterprise load laboratory in v1; targeted capture, ASR, diarization, and reconnect stress tests are required for their phases.
- No telemetry — debugging uses Tauri app-data logs (`%APPDATA%/com.mcnatg1.yap/logs/` on Windows).
