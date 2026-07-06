# Spec: Local LLM Sidecar

**Status:** Draft (2026-06-30)
**Implements:** [ADR 0005](../adr/0005-llama-server-agents.md), [ADR 0006](../adr/0006-silero-agents-state-machine.md) (HOT/background mutex)
**Scope:** Move Polish off the external Ollama dependency onto a bundled `llama-server` sidecar, and define the shared LLM client used by Polish today and Scribe/agents later.

Pairs with the [STT sidecar spec](local-live-fallback-sidecar.md) — same lifecycle/manager pattern, different runtime.

---

## 1. Scope

### In scope

- Bundle `llama-server` (llama.cpp) per platform; Rust sidecar manager.
- Pinned default GGUF for Polish/Scribe; first-run download + checksum.
- Migrate `desktop/src/polish.ts` from Ollama `/api/chat` to OpenAI-compatible `/v1/chat/completions`.
- `YAP_LLM_BACKEND=llama|ollama` flag; `YAP_LLM_BASE_URL` override.
- Shared LLM client (TS) + manager (Rust) reused by Scribe in Phase 3.

### Out of scope

| Deferred | Where |
|----------|-------|
| Live Scribe wiring | Phase D / [live spec](live-dictation-client-ux.md) |
| Background agents (Student…) | Phase 7d+ ([ADR 0006](../adr/0006-silero-agents-state-machine.md)) |
| Metal/CUDA offload | post-v1 (`-ngl 0` baseline) |
| Embeddings for RAG | [ADR 0011](../adr/0011-vector-rag-retrieval.md) |

---

## 2. Current state

`desktop/src/polish.ts` calls Ollama directly:

```
POST http://127.0.0.1:11434/api/chat
model "gemma4:e2b-it-q4_K_M", keep_alive "10m", options.num_gpu 0
```

Three tones (`light`/`clean`/`notes`) with per-tone instruction, temperature, `num_predict`. This logic is **preserved**; only the transport and process ownership change.

---

## 3. Runtime

| Piece | Value |
|-------|-------|
| Binary | `llama-server` from pinned llama.cpp; in Tauri resources per OS/arch |
| Port | `127.0.0.1:8081` (probe up to `8091` on conflict; held in Rust manager state) |
| Model | `~2B Q4_K` instruct GGUF, filename pinned in `desktop/llama-model.txt` |
| Cache | `YAP_MODELS_DIR` else `%LOCALAPPDATA%/Yap/models/` (shared with STT GGUF) |
| Launch | `llama-server -m <path> --host 127.0.0.1 --port 8081 -c 2048 -t 4 -ngl 0` |
| Alias | Served model alias `scribe`; weights bound by `-m` |

CPU-first on all platforms. Apple-Silicon Metal is a later ADR amendment, not a code branch now.

---

## 4. API contract (Rust manager + TS client → llama-server)

### 4.1 Health

```
GET /health  (or /v1/models)
200 → ready  → Setup shows "Polish engine ready"
```

### 4.2 Chat completion (Polish + Scribe)

```
POST /v1/chat/completions
{
  "model": "scribe",
  "messages": [ {"role":"system",...}, {"role":"user",...} ],
  "max_tokens": <profile>,
  "temperature": <profile>,
  "stream": false
}
200 → { "choices":[{"message":{"content":"..."}}], "usage":{...} }
```

- Polish maps the existing tone table to `max_tokens`/`temperature` (below).
- Streaming (`stream:true`, SSE) is used by **live Scribe** (Phase D) to honor the 400 ms budget; Polish stays non-streaming.

### 4.3 Token / latency budgets

| Caller | Phase | max_tokens | temperature | Budget |
|--------|-------|-----------|-------------|--------|
| Polish · light | A–B | 220 | 0.2 | none (user waits) |
| Polish · clean | A–B | 220 | 0.3 | none |
| Polish · notes | A–B | 320 | 0.3 | none |
| **Scribe (live)** | D | **120** | 0.2 | **≤400 ms wall** → else emit raw |

Live Scribe runs in the **HOT** mutex group (one at a time); if first token or completion exceeds 400 ms, abort and show raw ([ADR 0006](../adr/0006-silero-agents-state-machine.md)).

### 4.4 Scribe system prompt (pinned)

```
You are a private, on-device transcript cleanup engine.
Return ONLY the cleaned text — no preamble, no notes, never empty.
Preserve the speaker's meaning and voice. Do not summarize unless asked.
```

Tone-specific user-prompt instructions reuse the current `polishInstructions` strings verbatim. Store prompts under `desktop/src/agents/profiles/` (not inlined per ADR 0006).

---

## 5. Shared LLM client

- **TS** `llmClient.chat({ profile, messages })` — single place that knows base URL, backend flag, error mapping. `polish.ts` and future Scribe call this, not `fetch` directly.
- **Rust** manager owns spawn/health/restart/shutdown (mirrors STT manager); exposes a Tauri command for health + chosen port.
- Backend flag:

| `YAP_LLM_BACKEND` | Base URL | Use |
|-------------------|----------|-----|
| `llama` (default when healthy) | `http://127.0.0.1:8081/v1` | Shipped |
| `ollama` | `http://127.0.0.1:11434` (`/api/chat` shim) | Dev fallback |
| unset | try `llama`; unhealthy → `ollama` if present, else error toast | Migration |

---

## 6. Error codes

Reuse the catalog shape from the STT spec; LLM-specific:

| Code | Cause | UI message |
|------|-------|------------|
| `LLM_MODEL_MISSING` | GGUF not cached | “Polish model not downloaded.” → download |
| `LLM_UNREACHABLE` | server down / port conflict | “Polish engine isn’t responding.” → restart |
| `LLM_TIMEOUT` | exceeded budget (live Scribe) | (silent) → fall back to raw |
| `LLM_EMPTY` | empty completion | “Couldn’t clean that — try again.” (retry once with `stream:false`) |
| `LLM_OOM` | allocation failure | “Not enough memory for Polish.” |

Exhaustive `match` on the code enum.

---

## 7. Lifecycle (Rust manager)

Same rules as STT sidecar: lazy spawn on first Polish/Scribe use, `/health` ready gate (10 s), serialized HOT calls, auto-restart once, kill on app exit, logs at `%LOCALAPPDATA%/Yap/logs/llama-server.log`. Idle: keep model warm while app open; unload on app background only if RAM pressure (later). Coexists with STT sidecar within the 16 GB budget ([ADR 0004 §9](../adr/0004-background-diarization-okf-agents.md)).

---

## 8. Packaging

| Platform | Priority | Notes |
|----------|----------|-------|
| Windows x64 | **First** | Primary dev/target; CPU build |
| macOS arm64 | Second | Notarize bundled binary; CPU `-ngl 0` v1 |
| macOS x64 | If Intel retained | — |
| Linux x64 | Best-effort | — |

Pin llama.cpp version in repo; CI smoke-tests one chat completion per platform build.

---

## 9. Acceptance criteria

**Phase A — sidecar up**
- [ ] First run downloads + verifies Polish GGUF with progress.
- [ ] `/health` gates “Polish engine ready”.
- [ ] Sidecar starts/stops with app; no orphan on exit.

**Phase B — Polish migrated**
- [ ] All three tones produce non-empty output via `/v1/chat/completions`.
- [ ] Output quality parity with current Ollama path on a fixed sample (manual spot-check).
- [ ] `YAP_LLM_BACKEND=ollama` still works for dev.

**Phase C — default cutover**
- [ ] Default `llama`; Ollama only via flag; docs updated to dev-only.
- [ ] Error codes (§6) reachable and mapped to toasts.

**Phase D — Scribe-ready (with Live MVP)**
- [ ] Streaming completion honors 400 ms budget; over-budget falls back to raw and stores dual-track.
- [ ] Shared `llmClient` used by both Polish and Scribe (no duplicate fetch logic).

---

## 10. Open items

| Item | Resolve when |
|------|--------------|
| Exact GGUF (Gemma-2-2B vs Qwen2.5-1.5B vs Llama-3.2-3B) | Bench CPU latency vs Polish quality on target laptop |
| sha256 source for checksum | Picking the pinned HF revision |
| Whether to keep model warm or unload on background | After RAM profiling alongside STT sidecar |
