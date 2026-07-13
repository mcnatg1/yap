# ADR 0005: Bundled llama-server for LLM agents (CPU-first)

**Date:** 2026-06-30
**Status:** Accepted for the solo/local profile; team execution is amended by ADR 0014
**Builds on:** [ADR 0002](0002-crispasr-unified-stt-runtime.md) (sidecar pattern), [ADR 0004](0004-background-diarization-okf-agents.md) (Scribe agent, 400 ms bypass)
**Amended by:** [ADR 0014](0014-server-tier-compute-topology.md) — in the **team profile**, `llama-server` moves to the **LLM pool on the GB-class server node** (`-ngl` not 0; GPU-accelerated Scribe, polish, and agents). The **solo/local-first profile** retains the bundled `llama-server` sidecar with `--n-gpu-layers 0` as specified in this ADR. The CPU-first rule is the compatibility baseline for the solo profile; it does not apply to the server pool.

## Context

Yap uses a local LLM for **Polish** (transcript cleanup) and will use the same stack for future **Voice OS agents** (Scribe on the live path, Student/Curator/Librarian/Analyst off the hot path).

Today `desktop/src/polish.ts` calls **Ollama** at `http://127.0.0.1:11434/api/chat` with `keep_alive: "10m"` and **`num_gpu: 0`** (CPU-only polish).

We rejected relying on a **separate Ollama install** for shipped product UX. **Ollama uses llama.cpp under the hood** — bundling **`llama-server`** (llama.cpp’s OpenAI-compatible HTTP server) gives the same inference class with:

- One Yap installer (no “install Ollama first”)
- Models in `%APPDATA%/com.mcnatg1.yap/models/` (Windows) / `~/Library/Application Support/com.mcnatg1.yap/models/` (macOS)
- Same sidecar lifecycle pattern as CrispASR
- **CPU-first** on Windows and Mac (`--n-gpu-layers 0`); optional Metal on Apple Silicon later without changing architecture

CrispASR remains the **STT** runtime; llama-server is the **LLM agent** runtime. Do not conflate them.

## Decision

Adopt a **bundled `llama-server` sidecar** managed by Tauri (Rust) as the **primary LLM runtime** for Polish and all future agents.

### Runtime

| Piece | Choice |
|-------|--------|
| **Binary** | `llama-server` built from [llama.cpp](https://github.com/ggerganov/llama.cpp) — bundled per OS/arch in Tauri resources |
| **Process model** | Warm sidecar started with app (or on first LLM use); model stays loaded for session |
| **API** | OpenAI-compatible **`/v1/chat/completions`** on `127.0.0.1` (port TBD, e.g. `8081`) |
| **Dev fallback** | **`YAP_LLM_BACKEND=ollama|llama`** — Ollama at `:11434` for local dev until sidecar ships; not the shipped happy path |

### Default model (Scribe / Polish)

| Setting | Value |
|---------|--------|
| **GGUF** | Small instruct model, **~2B Q4_K** (e.g. Gemma-class e2b Q4 — exact file pinned in `desktop/llama-model.txt`) |
| **Path** | `%APPDATA%/com.mcnatg1.yap/models/<file>.gguf` |
| **Server load** | `-m <path>` at sidecar start; single model resident for v1 |
| **Context** | `-c 2048` (Scribe prompts are short) |
| **Threads** | `-t 4` default on 16 GB machines (cap to avoid starving CrispASR) |

### CPU-first (required)

All platforms must work **without a GPU**:

```text
llama-server -m .../gemma-2b-q4_k.gguf \
  --host 127.0.0.1 --port 8081 \
  -c 2048 -t 4 \
  -ngl 0
```

`-ngl 0` = all layers on CPU. Same flag on **Windows, macOS (Intel + Apple Silicon), Linux**.

**Optional later (not v1):** on Apple Silicon, `-ngl 99` or partial Metal for faster Scribe while staying local — still not cloud. CPU path remains the compatibility baseline.

### Scribe / Polish SLOs (unchanged from ADR 0004)

1. **Warm server** before live session or polish — cold load is not acceptable on hot path.
2. **400 ms budget** for live Scribe; exceed → show **raw STT**, polish async or skip.
3. **Dual-track storage** — save raw + polished text; user can revert to raw.
4. Background agents (Student, Analyst, …) share the same server with **lower priority** and no 400 ms cap.

### Integration

**Tauri (Rust):**

- Start/stop/health-check `llama-server` alongside `crispasr` sidecar.
- Setup status: **“Polish engine ready”** when server responds on `/v1/models` or health endpoint.
- Pin llama.cpp version in repo; CI smoke-test one chat completion.

**Frontend (`polish.ts` migration):**

- Replace Ollama `/api/chat` with OpenAI-compatible client:

```typescript
POST http://127.0.0.1:8081/v1/chat/completions
{
  "model": "scribe",
  "messages": [...],
  "max_tokens": 220,
  "temperature": 0.2
}
```

(Model name can be alias; actual weights loaded by server `-m` path.)

- Env: `YAP_LLM_BASE_URL`, `YAP_LLM_BACKEND`.

**Process layout (steady state):**

```
Yap (Tauri)
  ├─ crispasr sidecar        STT
  ├─ llama-server sidecar    LLM (Polish + agents)
  └─ yap-knowledge-worker    align/diarize (Phase 7)
```

### Model cache

Reuse the same directory as the local STT weights (`YAP_MODELS_DIR` / `%APPDATA%/com.mcnatg1.yap/models/` on Windows). First-run download or installer pre-cache for polish GGUF.

## Consequences

### Positive

- **Single-app UX** — no external Ollama dependency for end users.
- **Cross-platform** — llama.cpp is first-class on Windows, macOS, Linux.
- **CPU-first** — `-ngl 0` satisfies laptops without GPU; matches existing `num_gpu: 0` intent in polish.
- **Same sidecar pattern** as CrispASR — one Rust manager, predictable ops.
- **OpenAI API** — easy agent expansion (Scribe, Analyst) with one client.

### Negative

- **Build/signing** — ship `llama-server` per platform (like CrispASR).
- **Model UX** — we own download progress, checksums, disk space (Ollama did this for free).
- **CPU Scribe latency** — 2B Q4 warm is usually OK; 400 ms bypass still required on weak CPUs.
- **Migration** — rewrite `polish.ts` off Ollama API; dual backend during Phase 1.

### Neutral

- Performance at same quant/size **≈ Ollama** (same ggml core).
- Ollama remains valid for **developer machines** via `YAP_LLM_BACKEND=ollama`.

## Alternatives considered

### Keep Ollama as shipped runtime

**Rejected.** Extra install and split model store hurt UX; acceptable only as dev fallback.

### ONNX Runtime GenAI

**Rejected for v1.** Strong on fixed ONNX models, weaker model-swapping story; revisit if llama-server CPU insufficient.

### CrispASR speech-LLM backends for agents

**Rejected.** Wrong tool for glossary/RAG/chat agents (ADR 0004).

### In-process llama.cpp via Rust crate

**Deferred.** Possible later; sidecar matches CrispASR and isolates crashes.

## Implementation notes

### Phased rollout

| Phase | Scope |
|-------|--------|
| **A** | Bundle `llama-server`; Rust sidecar manager; health in Setup |
| **B** | Migrate `polish.ts` to `/v1/chat/completions`; feature flag |
| **C** | Default `YAP_LLM_BACKEND=llama`; Ollama dev-only docs |
| **D** | Live Scribe uses same server + 400 ms bypass (with Live MVP) |

### macOS notes

- Bundle **arm64** (required); **x86_64** if Intel Mac support retained.
- Notarize bundled binary with Tauri app.
- CPU: `-ngl 0` on all Macs for v1.
- Optional Metal acceleration: post-v1 ADR amendment.

### References

- Current Ollama polish: `desktop/src/polish.ts`
- Architecture: [VOICE-OS-ARCHITECTURE.md](../VOICE-OS-ARCHITECTURE.md)
- Agents: [ADR 0004](0004-background-diarization-okf-agents.md)
