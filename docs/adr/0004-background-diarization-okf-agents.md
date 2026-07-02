# ADR 0004: Background knowledge pipeline — diarization, micro-batches, OKF, and agents

**Date:** 2026-06-30
**Status:** Accepted (roadmap — Phase 7+ relative to current Yap; design validated against Voice OS spec)
**Builds on:** [ADR 0003](0003-long-term-voice-architecture.md) (recordings, LID, voice OS layers)
**Amended by:** [ADR 0015](0015-two-pass-diarization-speaker-identity.md) — the diarization design in this ADR (WeSpeaker ResNet34 + spectral clustering + Rolling Speaker Vault) is **retained for the solo/local-first profile** but **superseded for the team profile** by the two-pass ECAPA-TDNN + AHC/VBx pipeline. Specifically: WeSpeaker → ECAPA-TDNN; online k-NN replaces vault-first cosine matching; AHC + VBx replaces spectral clustering in Pass 2; the `yap-knowledge-worker` subprocess moves to the server-side KB compiler in the team profile. All of §3–§10 remain normative for the solo profile.

## Context

The **Voice OS** specification describes a **7-layer local-first system**: global hotkey dictation, async enrichment (forced alignment + speaker diarization), OKF markdown knowledge base, agentic glossary learning, MCP/vector gateways, and grounded Q&A.

A core engineering principle in that spec is **Critical Path Isolation**:

- **Layer 2 (real-time):** text on screen fast — no heavy math.
- **Layer 3 (background):** alignment, diarization, OKF — parallel, bounded RAM, no UI freeze.

The spec proposes a **modular CPU diarizer** (Silero VAD → WeSpeaker ResNet34 ONNX → spectral clustering) with **silence-anchored micro-batching**, a **Rolling Speaker Vault** for cross-chunk identity, and **multi-agent personas** (Scribe, Archivist, Student, etc.) with explicit **failure-state fallbacks**.

This ADR records how that pipeline **integrates with Yap ADRs 0001–0003**, what we adopt verbatim, what must change, and known weaknesses.

## Decision

### 1. Adopt Critical Path Isolation (unchanged principle)

| Layer | Runs when | Must never block |
|-------|-----------|------------------|
| **L2 Critical** | Live mic / ghost preview / inject | Diarization, alignment, OKF, agents |
| **L3 Background** | After chunk handoff or batch job completes | Live typing, CrispASR streaming |

Diarization and forced alignment live **only in L3**, never on the live hot path.

### 2. Reconciled critical path (vs Voice OS spec)

The Voice OS spec places **Cohere + SpeechBrain LID + Llama 3** on L2. **Accepted ADRs override** for Yap near-term:

| Voice OS spec (original) | Yap decision (0002–0003) | Notes |
|--------------------------|--------------------------|-------|
| Cohere on L2 live | **Moonshine streaming** (EN v1) on L2 | Cohere stays **recordings + re-pass** |
| `llama.cpp` STT cache | **CrispASR sidecar** warm GGUF | Same “warm process” idea |
| Llama 3 8B post-processor | **llama-server** (bundled; ~2B Q4 GGUF) — [ADR 0005](0005-llama-server-agents.md) | Scribe role; Ollama dev fallback |
| SpeechBrain LID on every utterance | **Off L2 in v1**; batch gate Phase 4+ | Avoid live latency |
| Silence chunker on L2 | **Adopt with non-blocking I/O** | See §3 |

**Reconciled L2 (Yap live / meeting capture):**

```
Mic → clean (WebRTC AGC optional) → Silero VAD
    → Moonshine streaming (CrispASR, -l en)     [live text]
    → llama-server polish (Scribe, optional, bypass if >400ms)
    → ghost UI / inject (future) / in-app panel (v1)

Parallel (non-blocking):
    VAD silence anchor → flush chunk → FIFO queue → L3 worker
```

**Reconciled L3 entry points (two sources):**

1. **Live session** — silence-anchored `.opus`/`.wav` micro-batches (30s+ after 1.5–2s pause).
2. **Batch recordings** — full file or chunked after Cohere transcribe completes (Yap queue).

Both feed the **same background worker**; batch files skip L2 chunker but still need alignment + diarization for speaker labels.

### 3. Silence-anchored micro-batching (adopt)

**Do not** slice audio on fixed 30s wall clock — cut on **VAD silence** to avoid mid-word boundaries.

| Parameter | Default | Rationale |
|-----------|---------|-----------|
| Min chunk duration | 30 s speech | Enough context for WeSpeaker + aligner |
| Silence trigger | 1.5–2.0 s continuous | Natural breath/pause boundary |
| Format | `.opus` preferred, `.wav` fallback | Disk vs decode cost |
| Queue | FIFO, bounded depth | Prevent backlog OOM on long meetings |

**Chunk payload handed to L3** (required fields — do not omit):

```json
{
  "chunk_id": "session-uuid-003",
  "session_id": "uuid",
  "audio_path": ".../chunk-003.opus",
  "text_raw": "...",
  "text_polished": "...",
  "t_start_ms": 90000,
  "t_end_ms": 122000,
  "language": "en",
  "source": "live|batch",
  "vad_segments": [[1200, 3400], [4100, 8900]],
  "degraded": false
}
```

- **`vad_segments`:** speech intervals from L2 Silero (ms relative to chunk). Worker **must not** re-run Silero when this array is non-empty.
- **`degraded`:** set `true` when queue back-pressure triggered (§10); worker may defer diarization until session end.

**Critical path rule:** chunk flush is **async disk write + queue push** only; never await alignment/diarization on L2.

**Chunk writer implementation:** dedicated **writer thread** with ring buffer; audio callback only enqueues PCM. Never `fsync` on the realtime thread.

### 4. Modular CPU diarizer (adopt, with vault)

**Pipeline per chunk (Math Engine B — parallel with alignment):**

```
Audio chunk
  → Silero VAD (speech frames) — reuse timestamps from L2 when available
  → WeSpeaker ResNet34 ONNX → 256-d embeddings per segment
  → Rolling Speaker Vault (cosine match vs session centroids, threshold ≥0.70)
  → optional spectral clustering only for unknown voices (1–15 cap)
  → speaker segments [{ t0, t1, speaker_id }]
```

**Rolling Speaker Vault (required — fixes chunk identity drift):**

| Step | Action |
|------|--------|
| Chunk 1 | Cluster or bootstrap → `SPEAKER_01`, `SPEAKER_02`; save centroids |
| Chunk N | Match new embeddings to vault; assign existing ID if sim ≥ 0.70 |
| New voice | Append `SPEAKER_XX` + centroid; cap at **15**; merge centroids when pairwise sim **≥0.85** |
| Session end | Persist vault + **stitch job** merges chunk OKF into one `conversations/<session>.md` |
| Display names | User may rename `SPEAKER_01` → `"Alex"`; stored in session metadata, not embedded in vault math |

**Diarization algorithm (fixed order):**

1. Extract WeSpeaker embeddings per VAD segment (min segment **500 ms**; shorter segments skipped).
2. Match each embedding to vault centroids (sim ≥ **0.70** → assign ID).
3. **Only unmatched** embeddings enter spectral clustering (similarity matrix truncated at **0.68**).
4. New clusters become new vault entries until cap 15.

**Batch shortcut:** recordings **under 5 minutes** skip micro-batch semantics — one manifest, whole-file align + diarize. Live sessions and files **≥5 min** use chunk pipeline.

### 5. Forced alignment + intersection (adopt)

**Math Engine A (parallel with diarizer):**

```
Audio chunk + transcript text
  → Wav2Vec2 / MMS forced aligner (CrispASR `canary-ctc-aligner` or wav2vec2 GGUF — TBD)
  → word-level [{ word, t0_ms, t1_ms }]
```

**Intersection (Majority Overlap Rule):**

- For each word `[t0, t1]`, find diarization segment with **>50% temporal overlap**.
- Assign `speaker_id`; unresolved → `SPEAKER_UNKNOWN`.

**Dependency:** Cohere/Moonshine output is **plain text without word timestamps** — alignment is **mandatory** before intersection, not optional.

**Text source for alignment:**

| Track | Use for alignment | Use for OKF display |
|-------|-------------------|---------------------|
| Raw STT | Primary align target | `transcript_raw` in frontmatter |
| Polished (Scribe) | Optional second pass | `transcript_display` default |

Align to **raw** first (faithful to audio). **Polished text inherits speaker labels** via shared word indices/timestamps — never run forced alignment on LLM output.

**Default aligner:** `canary-ctc-aligner` GGUF (multilingual subword) invoked from knowledge worker via ONNX or CrispASR CLI `-am` when sidecar exposes it; wav2vec2-en fallback for English-only fast path.

### 6. OKF output (adopt structure, simplify v1)

**Directories** (under user data, not repo):

```
%LOCALAPPDATA%/Yap/knowledge_base/
  conversations/      # timed, speaker-tagged transcripts
  jargon_glossary/    # term cards
  work_artifacts/     # todos, exports
  media_cache/        # opus/wav chunks
  quarantine/         # failed parses (see failure states)
```

**Markdown + YAML frontmatter** per conversation chunk or stitched session; wiki-links `[[Term]]` optional Phase 2.

Yap **Transcripts history** may mirror a subset (`conversations/`) before full OKF agent loop ships.

### 7. Background worker process model (decision)

**Use a separate child process for L3, not a Python `threading.Thread` in the Tauri or CrispASR process.**

| Option | Verdict |
|--------|---------|
| `threading.Thread` in app | **Rejected** — GIL contention; ONNX/torch/scikit-learn threads fight CrispASR live |
| **Dedicated `yap-knowledge-worker` subprocess** | **Accepted** — FIFO queue via IPC (stdin/JSON lines, named pipe, or localhost socket) |
| Same process as CrispASR | **Rejected** — couples STT latency to diarization |

**Thread caps inside worker only:**

```python
torch.set_num_threads(2)
ort.SessionOptions().intra_op_num_threads = 2
```

CrispASR sidecar retains its own thread budget; worker capped at **2–4 threads** on 16 GB machines.

**Process layout at steady state:**

```
Yap (Tauri + React)
  ├─ crispasr sidecar     (STT — hot)
  ├─ llama-server sidecar   (LLM — Polish + agents, warm)
  └─ yap-knowledge-worker (L3 — cold/warm, queue-driven)
```

### 8. Agent personas & failure states (adopt patterns)

Map Voice OS agents to implementation roles; all are **off critical path** except Scribe (optional on L2).

| Agent | Layer | Role | Failure fallback |
|-------|-------|------|------------------|
| **Scribe** | L2 | llama-server polish / filler strip | Dual-track raw+polished; **>400ms → raw only**; Cmd+Z restores raw |
| **Archivist** | L3 | OKF markdown writer | **Quarantine folder** — raw audio + JSON; worker continues |
| **Student** | L5 | Unknown term scanner | **Three-strike rule** + Ignore Forever blacklist |
| **Curator** | L5 | Glossary + wiki-links | **Local git** auto-commit before bulk edits; rollback |
| **Auditor** | L5 | Contradiction scan | Weekly cron; non-blocking notifications |
| **Librarian** | L7 | Hybrid retrieval | Confidence <0.60 → refuse context |
| **Analyst** | L7 | Grounded answer | Citations required; “no solid notes” template |
| **Coordinator** | L7 | Action items | High vs low confidence → To-Do vs Proposed Tasks |

Scribe dual-track aligns with existing Yap polish: store raw transcript beside polished in history.

### 9. Resource budget (16 GB RAM planning)

| Component | Budget | Notes |
|-----------|--------|-------|
| OS + Yap UI | ~2–3 GB | baseline |
| CrispASR (one GGUF) | ~0.2–1.5 GB | moonshine or cohere, exclusive |
| llama-server (Scribe GGUF) | ~1–2 GB | model dependent; `-ngl 0` |
| Knowledge worker peak | **<300 MB** | WeSpeaker ~100 MB + sklearn arrays |
| Queue depth | max **3** chunks buffered | exceeding triggers **degraded mode** (§10) |

**Reject Pyannote/NeMo full pipelines on 16 GB** for default path — spec’s modular stack stands.

### 10. Hardening requirements (weakness fixes — mandatory for L3 ship)

These promote former “room for improvement” items into **non-optional** implementation rules.

| Issue | Fix |
|-------|-----|
| Queue backlog | FIFO max **3**. On overflow: set `degraded: true` on new chunks; UI toast “Speaker labels will finish after session”; flush remaining at **session end** |
| Re-VAD waste | **`vad_segments` in manifest** (§3); worker skips Silero when provided |
| Sync disk on mic thread | **Ring buffer + writer thread** only (§3) |
| Vault vs re-cluster | **Vault-first** algorithm (§4); cluster unmatched only |
| Over-clustering noise | Min segment **500 ms**; ignore low-energy VAD slices |
| Worker hogs CPU | `BELOW_NORMAL` process priority (Windows) / `nice 10` (Unix); **2 ORT threads** max |
| Worker RAM leak | **Idle shutdown:** exit worker after **5 min** empty queue + no active session; Tauri restarts on next chunk |
| STT text drift | Store **`text_raw` per chunk** from same STT pass as live display; alignment always uses raw |
| OKF sprawl | Phase 7c writes OKF; until then append speaker tags to **existing Yap Transcripts JSON/history** |
| Curator git surprise | Git init **opt-in** in Settings; never silent |
| No observability | Local only: `%LOCALAPPDATA%/Yap/logs/knowledge-worker.log` — chunk ms, queue depth, vault size |
| Session stitch | **Archivist stitch job** at session end merges chunks + vault into one conversation file |

**IPC:** knowledge worker listens on `127.0.0.1` (port from env `YAP_KNOWLEDGE_PORT`) or named pipe; JSON-lines protocol; localhost-only.

## Consequences

### Positive

- Live stays responsive while meetings accumulate rich speaker-tagged OKF notes.
- Micro-batches flatten CPU/RAM vs 60-minute monolithic diarization.
- Speaker Vault solves the **#1 chunking diarization bug** (label swap between chunks).
- Quarantine + git fallbacks match local-first trust requirements.
- Same L3 worker serves **live sessions and dropped recordings**.

### Negative

- **Three child processes** (crispasr, llama-server, knowledge worker) — ops complexity; mitigated by idle worker shutdown and unified Setup health UI.
- Alignment wrong if user edits live text before chunk flush — mitigated by freezing **`text_raw` at chunk boundary** from STT stream, not from edited UI buffer.
- Vault thresholds may mis-merge similar voices — mitigated by user **speaker rename** and optional manual split in session review (future UI).
- OKF + agents scope is **large** — strict phase gating (§ Phased rollout); Yap batch+live ships first.

### Neutral

- Global OS injector / Caps Lock hotkey remains separate product surface (Phase 7+).
- MCP / vector DB gateway consume OKF output — no change to Yap core loop.

## Critical review (Voice OS spec vs this ADR)

### Strengths in the spec

1. **Critical Path Isolation** — correct; keep diarization out of L2.
2. **Silence-anchored chunking** — production-grade; avoids aligner/diarizer boundary bugs.
3. **Speaker Vault** — essential; without it, OKF multi-speaker notes are unusable.
4. **Lightweight diarizer** — WeSpeaker + sklearn vs Pyannote 8 GB — right for 16 GB targets.
5. **CPU thread caps** — necessary when CrispASR + worker coexist.
6. **Agent failure states** — dual-track Scribe, quarantine, RAG confidence gates — ship-worthy patterns.
7. **Optimistic background processing** — meeting ends with queue nearly drained.

### Weaknesses / gaps in the spec — status

| Issue | Status |
|-------|--------|
| Cohere on L2 live | **Fixed** — Moonshine L2 (ADR 0002) |
| LID on every utterance | **Fixed** — batch gate Phase 4; live EN v1 |
| Align polished text | **Fixed** — align raw only (§5) |
| Python thread worker | **Fixed** — subprocess (§7) |
| Sync chunk I/O | **Fixed** — writer thread (§3, §10) |
| Re-cluster every chunk | **Fixed** — vault-first (§4) |
| Queue back-pressure | **Fixed** — max 3 + degraded mode (§10) |
| Re-VAD in worker | **Fixed** — `vad_segments` manifest (§3) |
| OKF before history | **Fixed** — Transcripts first, OKF Phase 7c |
| Git on KB | **Fixed** — opt-in (§10) |

### Future enhancements (optional, post-7e)

- GPU aligner offload when CUDA available
- Per-room vault profiles (home office vs conference room)
- MCP gateway auto-publish on stitch complete

## Phased rollout (L3-specific)

| Phase | Scope |
|-------|--------|
| **7a** | Knowledge worker subprocess + FIFO; forced alignment only; plain timed transcript |
| **7b** | WeSpeaker + Speaker Vault + intersection; speaker tags in export |
| **7c** | OKF Archivist + `conversations/` layout |
| **7d** | Student / Curator / git (opt-in) |
| **7e** | Librarian / Analyst / MCP gateway |

Depends on ADR 0002 Phases 1–3 (CrispASR batch + live EN) being stable first.

## Alternatives considered

### Pyannote / NeMo diarization

**Rejected for default.** Quality strong but RAM/CPU incompatible with 16 GB + live STT concurrent target.

### Diarization on live critical path

**Rejected.** Violates isolation; causes dropouts and Wispr-class latency failure.

### Monolithic post-meeting processing only

**Rejected as sole strategy.** Acceptable fallback when queue back-pressure triggers; not primary UX for long meetings.

### Single-process Python for STT + L3

**Rejected.** GIL and thread pool contention (spec’s open question — answered here).

## References

- [ADR 0003](0003-long-term-voice-architecture.md) — LID, recordings, layer map
- [ADR 0002](0002-crispasr-unified-stt-runtime.md) — CrispASR sidecar
- CrispASR aligners: `canary-ctc-aligner`, wav2vec2 GGUF repos under `cstr/`
- Voice OS spec: Critical Path Isolation, Speaker Vault, agent roster (internal)
- Readable synthesis: [VOICE-OS-ARCHITECTURE.md](../VOICE-OS-ARCHITECTURE.md)
