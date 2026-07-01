# Design Spec: Phase 1 — CrispASR STT sidecar (batch)

**Status:** Draft (2026-06-30) — approved design, pre-implementation
**Refines:** ../../specs/phase-1-2-stt-sidecar.md
**Implements:** ../../adr/0001-dual-stt-backends.md, ../../adr/0002-crispasr-unified-stt-runtime.md, ../../adr/0006-silero-agents-state-machine.md

This spec **refines (does not replace)** the phase-1-2 STT sidecar spec: it narrows the work to a runnable Phase-1 slice, reconciles the IPC contract with the real CrispASR HTTP API (~v0.4.6), and adds a thorough security section. Style: terse ADR/spec — numbered sections + tables.

**Contents:**

1. Context & Current State (Phase 0)
2. Decision & Alternatives
3. Reconciled CrispASR API (~v0.4.6)
4. Scope — Three Buckets
5. Architecture
6. IPC Contract
7. Sidecar Lifecycle
8. Model Cache & Pinned Download/Verify
9. Binary Acquisition & Packaging
10. Security & Safety
11. Error Contract
12. Migration & Cutover
13. Settings / UI Surface
14. Acceptance Criteria
15. Testing (this cycle)
16. Explicitly Deferred
17. Resolved Decisions

## 1. Context & Current State (Phase 0)

Yap is a **local-first transcription** desktop app: **Tauri + Rust** backend, **React + shadcn** frontend. What ships today (Phase 0):

| Area | Today's behavior |
| --- | --- |
| Batch transcribe | Rust `transcribe_files` spawns `.venv/Scripts/python.exe transcribe.py <paths>` — **one process per call** — and parses output `.txt` paths from stdout. |
| Setup probe | `setup_status` only checks that `python.exe` + `transcribe.py` exist. |
| Python engine | `transcribe.py` loads **Cohere via HF Transformers**, writes a sibling `<stem>.txt` (CLI `--language en --no-punctuation --out-dir`). |
| Polish | `polish.ts` calls **Ollama** at `127.0.0.1:11434`. **UNCHANGED this phase.** |
| Everything else | All capability past the Phase-0 batch path is **docs-only** (not built). |

This phase changes *how batch STT runs* (adds the CrispASR sidecar as a selectable backend); it does **not** touch polish, live, or diarization.

## 2. Decision & Alternatives

**Decision:** Build Phase 1 of the CrispASR STT sidecar via **Approach A** — an **out-of-process HTTP sidecar** behind an internal Rust **`SttBackend` trait**, keeping the existing Python path as a **fallback backend**.

| # | Approach | Summary | Verdict |
| --- | --- | --- | --- |
| **A** | **HTTP sidecar** | Bundle a pinned `crispasr` binary, run `crispasr --server`, talk HTTP. | **CHOSEN** |
| B | In-process Rust binding (`libcrispasr` C-ABI) | Capability-complete: stable C-ABI; Rust `Session` incl. streaming `stream_open_ex(live:true)`, `feed()`, `get_text()`; `libcrispasr.dll` built in Windows CI; author ships CrisperWeaver on it. | Rejected (for now) |
| C | CLI-per-file | Simplest, but re-pays model load per file. | Rejected (ADR 0002) |

**Why A:** matches the ADR 0002 **warm-daemon** model; gives **process isolation + crash recovery**; is **uniform** with the future llama-server + knowledge-worker (all separate processes under the ADR 0006 orchestrator); and has the **best security posture** (a hardenable process boundary).

**Why not B:** a C++ ggml **crash / OOM / memory-safety bug** parsing untrusted GGUF/audio would take down the **whole app**; heavier per-platform link/sign; tracks a fast-moving binding API; shares the UI process.

**Why not C:** re-loading the ~1.2 GB model per file is exactly what ADR 0002 rejects.

**Low-regret note:** the trait lets us add a `CrispasrInProcess` backend **later, for the LIVE path only**, if latency measurements justify it — without disturbing the batch path.

## 3. Reconciled CrispASR API (~v0.4.6)

Verified against CrispASR **~v0.4.6**. This is the **real** surface and **differs from the old spec**.

**Server launch:** `crispasr --server -m <gguf> --host 127.0.0.1 --port <p>` (auto-warms the model in server mode).

| Method + path | Body | Response (shape) |
| --- | --- | --- |
| `GET /health` | — | `{"status":"ok","backend":"cohere"}` |
| `GET /backends` | — | `{"backends":[...],"active":"cohere"}` |
| `POST /inference` | multipart: `file=@audio` (+ optional `language`, `response_format`, …) | `{"text":"...","segments":[...],"backend":"cohere","duration":11.0}` |
| `POST /load` | multipart: `model=<gguf path>` | hot-swap; backend auto-detected; exclusive residency is inherent |
| `POST /v1/audio/transcriptions` | OpenAI-compatible | OpenAI transcription shape |

**Semantics:**
- Requests are **mutex-serialized server-side** (one inference at a time).
- Auto-download via `-m auto` / `--hf-repo OWNER/REPO[:FILE]` (curl, cached by filename); **mmap** supported. *(Phase 1 disables runtime download — see §8/§10.)*
- **cohere = 13 languages** (needs explicit `-l <code>`, or `-l auto` for an optional LID pre-step). **Old docs say 14 — that is wrong; we document 13 and flag it.**
- Windows build: `build-windows.bat` (VS 2022 via vswhere + Ninja) → `build\bin\crispasr.exe`; per-platform **prebuilt release binaries** also exist.
- **Batch GGUF:** `cohere-transcribe-q4_k.gguf` (~1.2 GB) from `cstr/cohere-transcribe-03-2026-GGUF`.

### 3.1 Deltas vs. old spec

| Old spec assumed | Reality (~v0.4.6) |
| --- | --- |
| `POST /transcribe` (single endpoint) | Two real endpoints — native `POST /inference` + OpenAI-compatible `POST /v1/audio/transcriptions`. **Happy path calls `/v1/audio/transcriptions`** (§6.2, §17); `/inference` is the documented alternative (§6.4). |
| JSON `{"audio_path": ...}` | **multipart** `file=@audio` upload |
| `/load` takes a **backend name** | `/load` takes a **model path** (GGUF) |
| `quant` is a per-request field | **No** per-request quant — Q8 is a *different GGUF* |
| `/health` rich payload | `/health` returns only `{status,backend}` → gate on **`status=="ok"` AND `active backend=="cohere"`** |
| language fixed at launch | per-request `language` via **form field** |
| batch progress via SSE | **SSE not guaranteed** — treat batch progress as **indeterminate** |

## 4. Scope — Three Buckets

| Bucket | Meaning | Items | Phase-1 action |
| --- | --- | --- | --- |
| **1** | **Design-for, DO NOT build** | live/moonshine WS + Silero VAD (Phase 3); chunk manifests / `vad_segments` (Phase 7); llama-server / polish migration (Phase A–D) | Accommodate via the trait; reserve `/load` for a future moonshine GGUF; share the models dir. |
| **2** | **IN SCOPE this cycle** | `SttError` enum + code→toast mapping (common codes now; exhaustive matrix later); port-conflict probe (**8765→8775**, held in manager state); idle-eviction (unload model after **10 min** batch idle); crispasr is the default (prefer-crispasr + auto-fallback to python), parity + crash-recovery is the trust bar | **Build.** |
| **3** | **YAGNI, design supports** | Q8 toggle (load 2nd GGUF via `/load`); GPU offload (`-ngl>0`, CPU-first); SSE batch progress | Leave hooks; do not implement. |

## 5. Architecture

`transcribe_files` becomes a **thin dispatcher** over an internal Rust **`SttBackend` trait** with two implementations. **Rust still writes the sibling `<stem>.txt`; the sidecar returns text only.**

```rust
// illustrative — not the final shipping signature
trait SttBackend {
    fn transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError>;
}
```

| Component | Role |
| --- | --- |
| `SttBackend` (trait) | Uniform batch API: audio path + language → text (or `SttError`). |
| `PythonBackend` | **Today's Phase-0 path, verbatim** (spawns `python.exe transcribe.py`). Fallback. |
| `CrispasrBackend` | Talks HTTP to the sidecar via the OpenAI-compatible **`POST /v1/audio/transcriptions`** (happy path, §6.2); **parses the response JSON tolerantly / shape-agnostically** — reads `text`, ignores unknown fields. |
| `CrispasrSidecar` (manager) | Owns the child process + endpoint state (§7). |
| Dispatcher | Selects a backend from `YAP_STT_BACKEND` (§12), calls `transcribe`, writes `<stem>.txt`. |

**`CrispasrSidecar` manager responsibilities:** lazy spawn on first use; `/health` **ready-gate (10 s → error)**; **one request in flight**; **restart-once + retry-file-once** on crash; **kill the child on app exit**.

**Backend selection:** `YAP_STT_BACKEND = crispasr | python | unset` (§12). `crispasr` **forces the HTTP sidecar; no fallback**; `python` **forces the legacy Phase-0 path**; **unset is the default and prefers `crispasr`** — try the sidecar, and **auto-fall-back to `python`** for the rest of the session if it is unhealthy at first use.

## 6. IPC Contract

The sidecar listens on `http://127.0.0.1:<port>` (§7). Concrete requests/responses (port `8765` shown as an example):

### 6.1 Readiness — `GET /health`

```
GET http://127.0.0.1:8765/health
→ 200 {"status":"ok","backend":"cohere"}
```

Ready **iff** `status=="ok"` **AND** `backend=="cohere"`.

### 6.2 Batch transcription (happy path) — `POST /v1/audio/transcriptions`

```
POST http://127.0.0.1:8765/v1/audio/transcriptions
Content-Type: multipart/form-data
  file=@C:\clips\meeting.wav
  language=en            # optional; omit or "auto" for the LID pre-step
  # OpenAI clients also send a `model` field; the sidecar uses the loaded GGUF regardless
→ 200 {"text":"..."}
```

`CrispasrBackend` **parses this JSON tolerantly / shape-agnostically**: it reads `text`, **ignores any unknown fields**, and Rust writes `text` to the sibling `<stem>.txt`. The OpenAI-compatible response **and** CrispASR both carry **segment + word timestamps** (the `cohere` backend has native word timing), so later phases can request `verbose_json` and read `segments`/word timestamps as a **small addition behind the existing `SttBackend` trait** — **no separate native `/inference` method is built now** (§6.4).

### 6.3 Model hot-swap — `POST /load`

```
POST http://127.0.0.1:8765/load
Content-Type: multipart/form-data
  model=%LOCALAPPDATA%\Yap\models\cohere-transcribe-q4_k.gguf
→ 200 (new model resident; backend auto-detected)
```

Used only for future Bucket-1/3 swaps (moonshine, Q8); **not** called on the happy batch path.

### 6.4 Native alternative — `POST /inference`

```
POST http://127.0.0.1:8765/inference
Content-Type: multipart/form-data
  file=@C:\clips\meeting.wav
  language=en            # optional; omit or "auto" for the LID pre-step
→ 200 {"text":"...","segments":[...],"backend":"cohere","duration":11.0}
```

A **documented alternative** to the happy-path `/v1/audio/transcriptions` (§6.2), kept for parity/debugging; **not called on the happy path** (decided — §17). Switching to it later (or reading its `segments`) is a small change behind the `SttBackend` trait.

## 7. Sidecar Lifecycle

| Concern | Policy |
| --- | --- |
| Spawn | **Lazy** — on the first `crispasr` transcription request. |
| Port | **Probe `8765 → 8775`**; the first free port is **held in manager state** for the session. |
| Launch | `crispasr --server -m <pinned gguf> --host 127.0.0.1 --port <port>` (scrubbed env, §10). |
| Ready-gate | Poll `GET /health` until `status=="ok"` AND `backend=="cohere"`; **10 s budget → error** (`SIDECAR_UNREACHABLE`). |
| Concurrency | **One request in flight** (mirrors the server-side mutex); an overlapping request returns `BUSY`. |
| Crash detection | Child exit or connection failure mid-request. |
| Restart policy | **Restart-once**, then **retry the file once**; a second failure → `SIDECAR_CRASH`. |
| Idle unload | After **10 min** of batch idle, unload the model to free RAM (re-spawn/re-load on next use). |
| App exit | **Kill the child** on app shutdown (no orphans). |
| Logs | `%LOCALAPPDATA%/Yap/logs/crispasr.log`. |

**Fallback scope:** the auto-fall-back to `python` on an unhealthy sidecar applies to the **unset/default (prefer-crispasr)** path only; a **forced** `YAP_STT_BACKEND=crispasr` surfaces the error instead (§11, §12).

## 8. Model Cache & Pinned Download/Verify

**Batch model:** `cohere-transcribe-q4_k.gguf` (~1.2 GB) from `cstr/cohere-transcribe-03-2026-GGUF`, **pinned by HF revision + SHA-256**.

| Step | Behavior |
| --- | --- |
| Location | `%LOCALAPPDATA%/Yap/models/` (override: **`YAP_MODELS_DIR`**). |
| First use | If the pinned GGUF is absent, download **the exact pinned revision** over HTTPS, then **verify SHA-256 before first use**. |
| Verify fail | **Fail closed** (`MODEL_CORRUPT`); never run an unverified artifact. |
| Runtime | After verification the sidecar runs **OFFLINE (egress blocked)** — this **supersedes** CrispASR runtime auto-download (`-m auto` / `--hf-repo`), which could fetch a mutable "latest". |
| Sharing | The dir is **shared** with future Bucket-1 models (e.g. moonshine) loaded via `/load`. |

**Download/verify flow (first use):**

1. Resolve the models dir (`YAP_MODELS_DIR`, else `%LOCALAPPDATA%/Yap/models/`).
2. If the pinned GGUF is present, **verify its SHA-256**; on match, use it.
3. If absent, **download the exact pinned HF revision over HTTPS**, then **verify SHA-256**.
4. On mismatch → `MODEL_CORRUPT` (**fail closed**); on match → mark cached and run the sidecar **offline**.

## 9. Binary Acquisition & Packaging

| Concern | Policy |
| --- | --- |
| Source | **Prebuilt release binary** from the **official GitHub releases** over HTTPS (preferred); `build-windows.bat` (VS 2022 via vswhere + Ninja → `build\bin\crispasr.exe`) as the fallback build path. |
| Version pin | **`desktop/crispasr-version.txt`** pins the exact version; the binary SHA-256 is verified on fetch **and re-verified on launch**. **No silent auto-update.** |
| Packaging | Ship the verified binary as a **Tauri `externalBin` sidecar**; resolve the per-OS (target-triple) sidecar path at runtime. |
| Dev override | **`YAP_CRISPASR_BIN`** points at a local build during development. |

**Binary resolution order:** `YAP_CRISPASR_BIN` (dev) → bundled `externalBin` sidecar for the current OS/target-triple → error (`SIDECAR_UNREACHABLE`). The resolved binary's SHA-256 is checked against the pin in `desktop/crispasr-version.txt` **before spawn**.

**Distribution signing (REQUIRED before ship):** the packaged app **and its nested `crispasr` sidecar** must be **code-signed + notarized** before distribution (see §10.4). CrispASR ships no code-signing / SLSA provenance, so our pinned SHA-256 (§8, §10.3) is the **primary integrity gate** and signing covers the shipped bundle. **Not needed to run the local Phase-1 dev slice.**

## 10. Security & Safety

Adding CrispASR expands the attack surface. This section is a **user priority** and is treated as required Phase-1 work.

### 10.1 Threat model

On top of the existing app we add:
- **(a) A memory-UNSAFE native C++ ggml runtime** parsing **untrusted inputs** (user-dropped audio + a downloaded GGUF) — a parsing bug is a potential RCE/OOM.
- **(b) Third-party HF model artifacts** (the GGUF) — supply-chain trust in the artifact itself.
- **(c) The existing JS/Rust build supply chain** — worm class: **npm "Shai-Hulud"** — malicious `install`/`postinstall` scripts that **harvest dev/CI secrets and self-propagate via stolen tokens**.

### 10.2 Why Approach A is safer (two dimensions)

| Dimension | In-process (B) | Out-of-process sidecar (A) |
| --- | --- | --- |
| **Build-time code execution** | A crate `build.rs` **re-introduces arbitrary build-time code** where tokens live. | A **pinned, hash-verified prebuilt binary runs NO code during `npm`/`cargo` build** — it is a **data asset**. **→ favors A** |
| **Runtime blast radius** | An in-process C++ **RCE runs with the full app's privileges**. | **Neither is sandboxed by default** (*a separate process is only a boundary if hardened*), but the sidecar **is hardenable** — restrict privileges/network/filesystem, observe + kill it. **→ favors A** |

### 10.3 Controls — Phase-1 REQUIRED

| Control | Why |
| --- | --- |
| Pin `crispasr` version (`desktop/crispasr-version.txt`) + verify binary **SHA-256**; fetch from **official GitHub releases over HTTPS**; **re-verify on launch**; **no silent auto-update**. | The binary is the native-code trust anchor; pin+hash blocks tamper/downgrade. |
| Pin the GGUF by **HF revision + SHA-256**; download the exact pinned artifact + **verify before first use**; then run the sidecar **OFFLINE (egress blocked)**. | Supersedes runtime auto-download of a mutable "latest"; freezes the model we vetted. |
| Spawn the sidecar with a **SCRUBBED environment** (no inherited HF/cloud/GitHub tokens). | A compromised native runtime can't exfiltrate secrets that aren't in its env. |
| Bind **`127.0.0.1` ONLY** (never `0.0.0.0`); **ephemeral port** held in manager state; **optional per-session shared-secret header** on requests. | Keeps the STT surface off the network and un-addressable by other local apps. |
| Treat sidecar **output as untrusted**; **validate/limit inputs** (path, size). | Defends the Rust side against malformed/oversized responses and path abuse. |
| **CI supply-chain hygiene:** commit + honor lockfiles (**`npm ci`**, **`cargo --locked`**); prefer **`npm install --ignore-scripts`**; keep Tauri **`capabilities/default.json` minimal** (least privilege); enable **`npm audit` / `cargo audit` / Dependabot**. | Directly counters the Shai-Hulud install-script worm and least-privileges the app. |

### 10.4 Distribution signing (REQUIRED before ship) & fast-follow

**REQUIRED before distribution — code-sign + notarize the app INCLUDING the nested sidecar binary** (Windows **Authenticode** via **Azure Trusted Signing**; macOS notarization). **Elevated from a fast-follow nice-to-have to a required-before-distribution gate.** **Why:** CrispASR ships **no code-signing or SLSA provenance**, so our **pinned SHA-256 (§10.3) is the primary integrity gate**, and signing covers the **shipped bundle** (app + nested `crispasr` sidecar) against tamper on the distribution channel. **NOT required to run the Phase-1 dev slice locally; REQUIRED before shipping to users.** *(Binary/integrity calls validated via web research 2026-07-01.)*

**Fast-follow (still not required in Phase 1):**

- **OS-sandbox the sidecar** (Windows Job Object/AppContainer, macOS sandbox profile, Linux seccomp/namespaces) restricting FS to **models + temp** and **blocking network**.
- Optionally run the sidecar under a **lower-privilege token**.

### 10.5 Security acceptance

- [ ] **Hash verification fails closed** (a bad binary or GGUF → refuse to run).
- [ ] Sidecar observed **listening on loopback only**.
- [ ] Sidecar **env contains no secrets**.
- [ ] **Python fallback still works.**

## 11. Error Contract

`SttError` is a Rust enum mapped to UI toasts. **Rust must handle every variant with an exhaustive `match` (no catch-all)** — this repo enforces exhaustive match handling; the TS code→toast map is likewise exhaustive (`never` default).

| Code | Cause | UI message (toast) | Recovery |
| --- | --- | --- | --- |
| `MODEL_MISSING` | Pinned GGUF absent, no cached copy | "Transcription model isn't installed yet." | Trigger the pinned download (§8); point to Setup. |
| `MODEL_CORRUPT` | SHA-256 mismatch | "Model file failed verification." | **Fail closed**; delete + re-download the pinned artifact. |
| `BAD_LANG` | Language not one of the **13** cohere codes | "That language isn't supported." | Retry with `auto`/English; show the supported list. |
| `OOM` | Model/audio exceeds memory | "Ran out of memory while transcribing." | Idle-unload; suggest closing apps / a smaller batch. |
| `AUDIO_DECODE` | Input audio can't be decoded | "Couldn't read that audio file." | Skip the file; continue the batch. |
| `SIDECAR_CRASH` | Child crashed after restart-once + retry-file-once | "Transcription engine crashed." | On unset/default, **auto-fall-back to `python`** for the session; on forced `crispasr`, surface + offer retry. |
| `SIDECAR_UNREACHABLE` | Ready-gate 10 s timeout / connect fail | "Transcription engine didn't start." | On unset/default, **auto-fall-back to `python`** for the session; on forced `crispasr`, surface (check `crispasr.log`) + offer retry. |
| `BUSY` | Request while one is in flight | "Transcription is busy — try again in a moment." | Retry when the in-flight request finishes. |
| `TIMEOUT` | Inference exceeded the time budget | "Transcription timed out." | Retry the file once. |

**Fallback vs. surface:** on the **unset/default (prefer-crispasr)** path, `SIDECAR_UNREACHABLE` / `SIDECAR_CRASH` **auto-fall-back to `python`** for the rest of the session (logged loudly + shown in Setup); a **forced** `YAP_STT_BACKEND=crispasr` **surfaces the error instead** (no fallback).

The **`match` over `SttError` is exhaustive now** (every variant handled). Per Bucket 2, the mapped set of *distinct real-world causes* starts with these common ones; a fully exhaustive cause→recovery matrix is a later refinement.

## 12. Migration & Cutover

**Backend selection** resolves from `YAP_STT_BACKEND`:

| `YAP_STT_BACKEND` | Backend used |
| --- | --- |
| `crispasr` | `CrispasrBackend` — **force the HTTP sidecar; no fallback** (an unhealthy sidecar surfaces `SIDECAR_UNREACHABLE` / `SIDECAR_CRASH`). |
| `python` | `PythonBackend` — **force the legacy Phase-0 path**. |
| _unset_ | **Default — prefer `crispasr`:** try the sidecar; if it is unhealthy at first use, **auto-fall-back to `python`** for the rest of the session (logged + shown in Setup). |

**`crispasr` is preferred from day one; there is no later default flip.** On the unset/default path an unhealthy sidecar (binary/GGUF missing, not ready within the 10 s ready-gate, or a crash after restart-once + retry-file-once) **transparently falls back to `python`** for the rest of the session, logs it loudly, and reflects it in the Setup status. `python` **remains the documented fallback** indefinitely.

**Practical reality (today):** until the pinned `crispasr` binary + GGUF are obtained and SHA-verified (§8, §9), "prefer `crispasr`" will transparently fall back to `python` on every batch — **that is the backup doing its job**, not a regression.

### 12.1 Trust bar (when to rely on crispasr)

`crispasr` is already the preferred default; these criteria are the **trust bar** — the point at which we **rely on `crispasr`** and call Phase 1 done, so a `python` fallback becomes a **real incident** rather than expected. **They are not a trigger to change the default.** Both must hold:

- [ ] **Batch parity spot-check passes** (WER-tolerant vs the Python path on a known clip — §15).
- [ ] **Crash-recovery works** (restart-once + retry-file-once — §7).

Until both pass, an occasional `python` fallback is **expected**, not an incident.

**Rollback:** if a regression appears, **force `YAP_STT_BACKEND=python`** to pin the Phase-0 path with no rebuild; the fallback is always available.

## 13. Settings / UI Surface

| Surface | Behavior |
| --- | --- |
| Setup readiness | Show **"Transcription engine ready"** when `/health` is ok **AND** the pinned model is cached. |
| Fallback status | When the **unset/default** path has fallen back, Setup shows a terse **"Using Python fallback"** note (§12). |
| Download | **Inline download progress** for the pinned GGUF (indeterminate is acceptable — no guaranteed SSE). |
| Errors | Surface `SttError` as **toasts** (§11). |
| Hygiene | **No raw binary names or ports** on the primary UI (internals stay in logs / Setup detail). |

## 14. Acceptance Criteria

**Functional**
- [ ] `crispasr` backend transcribes a known clip; **Rust writes the sibling `<stem>.txt`** (sidecar returns text only).
- [ ] **WER-tolerant parity** vs the Python path on the known clip (§15).
- [ ] Lazy spawn + **`/health` ready-gate**; 10 s timeout → `SIDECAR_UNREACHABLE`.
- [ ] **One request in flight**; crash → **restart-once + retry-file-once**.
- [ ] **Idle unload** after 10 min of batch idle.
- [ ] **Child killed on app exit** (no orphans).
- [ ] `YAP_STT_BACKEND` selects the backend: `crispasr` forces the sidecar (no fallback), `python` forces the Phase-0 path.
- [ ] **Unset default prefers `crispasr` and auto-falls-back to `python`** when the sidecar is unhealthy (logged + shown in Setup).

**Security** (from §10.5)
- [ ] Hash verification **fails closed**.
- [ ] Sidecar listens on **loopback only**.
- [ ] Sidecar **env has no secrets**.
- [ ] **Python fallback still works.**

## 15. Testing (this cycle)

| Test | Scope |
| --- | --- |
| Parity spot-check | **WER-tolerant** comparison of `crispasr` vs the **Python path** on a **known clip** (feeds the trust bar, §12.1). |
| Backend dispatch | A **pure Rust unit test** for backend selection/dispatch (`YAP_STT_BACKEND` → correct impl). |
| CI smoke matrix | **Deferred** (not built this cycle). |

## 16. Explicitly Deferred

**Bucket 1 (design-for, not built):** live/moonshine WS + Silero VAD (Phase 3); chunk manifests / `vad_segments` (Phase 7); llama-server / polish migration (Phase A–D). *Accommodated via the trait; `/load` + shared models dir reserved.*

**Bucket 3 (YAGNI, design supports):** Q8 toggle (2nd GGUF via `/load`); GPU offload (`-ngl>0`, CPU-first); SSE batch progress. *Hooks left; not implemented.*

## 17. Resolved Decisions

The four former open items are **resolved and FINAL** (endpoint, binary, and integrity calls validated via web research 2026-07-01). The only things still filled in **at build time** are genuine values — the exact pinned version string and the SHA-256 hashes — not placeholders.

1. **Endpoint — FINAL:** `CrispasrBackend` calls the OpenAI-compatible **`POST /v1/audio/transcriptions`** on the batch happy path (§3.1, §5, §6.2). Web-confirmed that this response shape **and** CrispASR carry **segment + word timestamps** (the `cohere` backend has native word timing), so the choice is **safe for Phase 7 alignment/diarization**. **No separate native `/inference` method is built now**; native `/inference` stays a **documented alternative** (§6.4), and reading word/segment timestamps (via `verbose_json`) later is a **small addition behind the existing `SttBackend` trait**.
2. **Model fetch — FINAL:** **Rust-owned pinned download + SHA-256 verify**; the sidecar runs **offline after verify** (§8). This supersedes CrispASR's runtime auto-download, so the exact `--hf-repo` / model-cache flag spelling is **moot on the happy path** — Rust fetches the pinned revision itself.
3. **Binary — FINAL:** ship a **pinned prebuilt official-release binary** as a Tauri **`externalBin` sidecar**; **cross-check GitHub's per-asset SHA-256 at pin time**; `build-windows.bat` remains the **documented fallback** build path (§9).
4. **Integrity anchor — FINAL:** pin **our own SHA-256 for BOTH the binary and the GGUF** in **`desktop/crispasr-version.txt`**, **fail-closed** on mismatch (§8, §9, §10.3); enable **`npm audit` / `cargo audit`** (§10.3). CrispASR ships no code-signing or SLSA provenance, so **our pinned SHA-256 is the primary integrity gate**, and **signing the shipped bundle is required before distribution** (§10.4).

**Pinned at build time (genuine values, not placeholders):** the exact `crispasr` version string and the SHA-256 hashes for the binary and the GGUF, recorded in `desktop/crispasr-version.txt`.
