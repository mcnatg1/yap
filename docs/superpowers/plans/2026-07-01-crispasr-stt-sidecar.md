# CrispASR STT Sidecar (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a pinned, hash-verified CrispASR HTTP sidecar as the preferred batch STT backend behind an internal Rust `SttBackend` trait, keeping the existing Python `transcribe.py` path as an automatic fallback.

**Architecture:** `transcribe_files` becomes a thin dispatcher that selects a backend from `YAP_STT_BACKEND` over an `SttBackend` trait. `CrispasrBackend` talks HTTP to a lazily-spawned, loopback-only `crispasr --server` child process owned by a `CrispasrSidecar` manager; `PythonBackend` wraps today's `transcribe.py` verbatim. Rust owns model + binary SHA-256 verification (fail-closed) and writes the sibling `<stem>.txt`; the sidecar returns text only.

**Tech Stack:** Rust (Tauri v2, `reqwest` blocking + multipart/json, `sha2`), TypeScript/React (`sonner` toasts, `vitest`), CrispASR ~v0.4.6 GGUF (`cohere` backend).

---

## Source of Truth

This plan implements Phase 1 of `docs/superpowers/specs/2026-06-30-crispasr-stt-sidecar-design.md`. Section references (§) point at that spec. Scope is the spec's **§4 Bucket 2** only.

## Design Notes & Trade-offs (read before Task 1)

- **Per-file trait, batch-preserving fallback.** `SttBackend::transcribe` is per-file (matches the spec's illustrative signature, §5). A default `transcribe_batch` loops per-file (used by `CrispasrBackend` against the warm sidecar). `PythonBackend` **overrides `transcribe_batch`** to spawn `transcribe.py` once for the whole batch, so the Python fallback keeps its single model-load (no Phase-0 regression).
- **Language.** The dispatcher passes `"en"` (today's Phase-0 default; `transcribe_files` takes only `paths`). A language picker is out of scope; `BAD_LANG` is still defined + mapped for exhaustiveness and future use.
- **Sidecar spawn uses `std::process::Command`** (not `tauri-plugin-shell`) so Rust fully controls the scrubbed environment and performs the pre-spawn binary SHA-256 check. Consequence: **no new capability permission** is added (least privilege, §10.3).
- **Blocking HTTP, sync command (matches today).** `transcribe_files` stays a synchronous `#[tauri::command]` using `reqwest`'s blocking client — identical threading to the current Phase-0 command, which already blocks on the Python subprocess. Per Tauri v2, sync commands run on the main thread, so a batch blocks the UI while it runs (exactly as today); no bespoke async runtime is added, and `reqwest::blocking` is safe here (the main thread is not inside a Tokio context). Making the command fully async (`async fn` + `spawn_blocking` with owned `Arc` handles, to dodge the `State<'_>` lifetime limitation) is a deliberate out-of-scope fast-follow.
- **Shared-secret header deferred.** The spec lists it as *optional* (§10.3); CrispASR ~v0.4.6 does not enforce a request secret, so a header it ignores would be security theater. The **enforced** controls are loopback-only bind + ephemeral port. Revisit if the sidecar gains secret support.

## Out of Scope (note once)

- **Bucket 1 (design-for, DO NOT build):** live/moonshine WS + Silero VAD (Phase 3); chunk manifests / `vad_segments` (Phase 7); llama-server / polish migration (Phase A–D). Accommodated only via the trait + a reserved `/load` path + the shared models dir.
- **Bucket 3 (YAGNI):** Q8 toggle (2nd GGUF via `/load`); GPU offload (`-ngl>0`); SSE batch progress. Hooks left; not implemented.
- **Release fast-follow (§10.4), NOT built here:** OS-sandboxing the sidecar (Job Object/AppContainer/seccomp) and code-signing + notarizing the app **and** its nested `crispasr` sidecar. These are **required before shipping to users** but not needed to run the Phase-1 dev slice; our pinned SHA-256 (§10.3) is the primary integrity gate in the meantime.

## File Structure

**Created — Rust (`desktop/src-tauri/src/stt/`), one responsibility each:**

| File | Responsibility |
| --- | --- |
| `mod.rs` | Module wiring + re-exports only. |
| `error.rs` | `SttError` enum + `code()` / `user_message()` — the error contract (§11). |
| `pin.rs` | `CrispasrPin` + `parse_pin` — parse/validate `crispasr-version.txt` (§9, §17). |
| `model.rs` | Models dir resolution, SHA-256 verify, pinned GGUF download (fail-closed) (§8). |
| `backend.rs` | `SttBackend` trait + `BackendChoice` + `select_backend` (§5, §12). |
| `python.rs` | `PythonBackend` — verbatim `transcribe.py` wrapper / fallback (§5). |
| `crispasr.rs` | `CrispasrBackend` HTTP client + tolerant JSON parse + input validation (§6.2, §10.3). |
| `sidecar.rs` | `CrispasrSidecar` manager: port probe, scrubbed-env spawn, health gate, restart-once, idle-unload, kill; binary resolution + verify (§7, §9, §10). |
| `parity.rs` | `word_error_rate` + `parse_verbose_json_has_timestamps` — helpers feeding the parity test (§12.1, §15). |
| `dispatch.rs` | `transcribe_paths` dispatcher, `TranscriptResult`, `SttCommandError`, `SttState`, engine-status labels (§5, §12, §13). |

**Modified — Rust:**

| File | Change |
| --- | --- |
| `desktop/src-tauri/src/lib.rs` | `mod stt;`; register `SttState`; rewrite `transcribe_files` + `setup_status` to delegate; kill child on exit; start idle monitor. |
| `desktop/src-tauri/Cargo.toml` | Add `reqwest`, `sha2`. |

**Created / Modified — config, frontend, tests, CI:**

| File | Change |
| --- | --- |
| `desktop/crispasr-version.txt` | **Create** — pinned version + binary/GGUF SHA-256 + HF repo/revision/file. |
| `desktop/src-tauri/tauri.conf.json` | **Modify** — add `bundle.externalBin`. |
| `desktop/src-tauri/capabilities/default.json` | **Verify unchanged** — least-privilege (no shell permission). |
| `desktop/src/stt.ts` | **Create** — `SttErrorCode`, `sttErrorMessage` (exhaustive `never`), `SttInvokeError`, `transcribeFiles` (mirrors `polish.ts`). |
| `desktop/src/stt.test.ts` | **Create** — vitest test for the mapping. |
| `desktop/src/App.tsx` | **Modify** — call `transcribeFiles`; surface batch + per-file errors. |
| `desktop/package.json` | **Modify** — add `vitest` + `test` script. |
| `desktop/src-tauri/tests/parity.rs` | **Create** — gated WER parity + `verbose_json` timestamp probe (§15). |
| `.github/workflows/ci.yml` | **Create** — `npm ci`, `cargo build --locked`, `npm audit`, `cargo audit` (§10.3). |
| `.github/dependabot.yml` | **Create** — npm + cargo update PRs (§10.3). |

---

## Tasks

### Task 1: Dependencies & `stt` module scaffold

**Files:**
- Modify: `desktop/src-tauri/Cargo.toml`
- Create: `desktop/src-tauri/src/stt/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`

> Infrastructure task — verified by a build instead of a unit test. All Rust commands in this plan run from `desktop/src-tauri/`; `git`/`npm` commands run from the repo root.

- [ ] **Step 1: Add the Rust HTTP + hashing dependencies**

Add these two lines to the `[dependencies]` table in `desktop/src-tauri/Cargo.toml`:

```toml
reqwest = { version = "0.12", features = ["blocking", "multipart", "json"] }
sha2 = "0.10"
```

- [ ] **Step 2: Create the `stt` module root**

Create `desktop/src-tauri/src/stt/mod.rs`:

```rust
//! STT backends: dispatcher, error contract, Python fallback, and the CrispASR
//! HTTP sidecar. Spec: docs/superpowers/specs/2026-06-30-crispasr-stt-sidecar-design.md
```

- [ ] **Step 3: Declare the module in the crate**

Add this line at the very top of `desktop/src-tauri/src/lib.rs` (above the first `#[tauri::command]`):

```rust
mod stt;
```

- [ ] **Step 4: Verify it builds**

Run (in `desktop/src-tauri/`): `cargo build`
Expected: PASS. The first build downloads and compiles `reqwest` + `sha2`; the empty `stt` module compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock desktop/src-tauri/src/lib.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "chore(stt): add reqwest/sha2 deps and stt module scaffold"
```

### Task 2: `SttError` enum + code/message contract

**Files:**
- Create: `desktop/src-tauri/src/stt/error.rs`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

The error contract from §11: nine variants, each with a stable `code()` (sent to the frontend) and a `user_message()` (toast text). Every `match` over `SttError` in this plan is exhaustive (no `_`).

- [ ] **Step 1: Write the failing test**

Add `pub mod error;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/error.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::SttError;

    #[test]
    fn every_variant_has_stable_code_and_message() {
        let all = [
            SttError::ModelMissing,
            SttError::ModelCorrupt,
            SttError::BadLang,
            SttError::Oom,
            SttError::AudioDecode,
            SttError::SidecarCrash,
            SttError::SidecarUnreachable,
            SttError::Busy,
            SttError::Timeout,
        ];
        for error in all {
            assert!(!error.code().is_empty());
            assert!(!error.user_message().is_empty());
        }
        assert_eq!(SttError::SidecarUnreachable.code(), "SIDECAR_UNREACHABLE");
        assert_eq!(SttError::ModelCorrupt.user_message(), "Model file failed verification.");
        assert_eq!(SttError::Timeout.to_string(), "TIMEOUT: Transcription timed out.");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::error`
Expected: FAIL — `cannot find type SttError in this scope`.

- [ ] **Step 3: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/error.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttError {
    ModelMissing,
    ModelCorrupt,
    BadLang,
    Oom,
    AudioDecode,
    SidecarCrash,
    SidecarUnreachable,
    Busy,
    Timeout,
}

impl SttError {
    pub fn code(&self) -> &'static str {
        match self {
            SttError::ModelMissing => "MODEL_MISSING",
            SttError::ModelCorrupt => "MODEL_CORRUPT",
            SttError::BadLang => "BAD_LANG",
            SttError::Oom => "OOM",
            SttError::AudioDecode => "AUDIO_DECODE",
            SttError::SidecarCrash => "SIDECAR_CRASH",
            SttError::SidecarUnreachable => "SIDECAR_UNREACHABLE",
            SttError::Busy => "BUSY",
            SttError::Timeout => "TIMEOUT",
        }
    }

    pub fn user_message(&self) -> &'static str {
        match self {
            SttError::ModelMissing => "Transcription model isn't installed yet.",
            SttError::ModelCorrupt => "Model file failed verification.",
            SttError::BadLang => "That language isn't supported.",
            SttError::Oom => "Ran out of memory while transcribing.",
            SttError::AudioDecode => "Couldn't read that audio file.",
            SttError::SidecarCrash => "Transcription engine crashed.",
            SttError::SidecarUnreachable => "Transcription engine didn't start.",
            SttError::Busy => "Transcription is busy — try again in a moment.",
            SttError::Timeout => "Transcription timed out.",
        }
    }
}

impl std::fmt::Display for SttError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.user_message())
    }
}

impl std::error::Error for SttError {}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::error`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/error.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): add SttError contract with codes and toast messages"
```

### Task 3: Version pin file + `CrispasrPin` parser

**Files:**
- Create: `desktop/src-tauri/src/stt/pin.rs`
- Create: `desktop/crispasr-version.txt`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

Pins the exact `crispasr` version + the binary/GGUF SHA-256 + the HF coordinates (§9, §17). The parser is embedded at compile time via `include_str!`, so the file must exist before the code compiles, and it can never carry a malformed hash (fail-closed).

- [ ] **Step 1: Create the pin file with genuine values**

Create `desktop/crispasr-version.txt`. Fill the three build-time values (`binary_sha256`, `gguf_revision`, `gguf_sha256`) — these are **genuine values required by §17, not placeholders**. Produce them with the commands below, then paste the results:

```ini
# CrispASR Phase-1 pins. All values are integrity-critical; verification is fail-closed.
crispasr_version=0.4.6
binary_sha256=<Get-FileHash SHA256 of the pinned crispasr.exe, lowercase 64-hex>
gguf_repo=cstr/cohere-transcribe-03-2026-GGUF
gguf_revision=<full 40-char HF commit hash to pin>
gguf_file=cohere-transcribe-q4_k.gguf
gguf_sha256=<Get-FileHash SHA256 of the downloaded gguf, lowercase 64-hex>
```

Commands to produce the genuine values (PowerShell):

```powershell
# gguf_revision: open the repo's commit history and copy the newest full commit hash:
#   https://huggingface.co/cstr/cohere-transcribe-03-2026-GGUF/commits/main
# gguf_sha256: download that exact revision, then hash it:
$rev = "<the commit hash you pinned above>"
Invoke-WebRequest -Uri "https://huggingface.co/cstr/cohere-transcribe-03-2026-GGUF/resolve/$rev/cohere-transcribe-q4_k.gguf" -OutFile cohere-transcribe-q4_k.gguf
(Get-FileHash -Algorithm SHA256 .\cohere-transcribe-q4_k.gguf).Hash.ToLower()
# binary_sha256: hash the pinned crispasr.exe (a GitHub release asset, or build-windows.bat output):
(Get-FileHash -Algorithm SHA256 .\crispasr.exe).Hash.ToLower()
```

- [ ] **Step 2: Write the failing test**

Add `pub mod pin;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/pin.rs` with only the test (the `SAMPLE` hashes are synthetic test fixtures, not the real pin):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# comment line
crispasr_version=0.4.6
binary_sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
gguf_repo=cstr/cohere-transcribe-03-2026-GGUF
gguf_revision=1111111111111111111111111111111111111111
gguf_file=cohere-transcribe-q4_k.gguf
gguf_sha256=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
";

    #[test]
    fn parses_valid_pin() {
        let pin = parse_pin(SAMPLE).unwrap();
        assert_eq!(pin.crispasr_version, "0.4.6");
        assert_eq!(pin.gguf_repo, "cstr/cohere-transcribe-03-2026-GGUF");
        assert_eq!(pin.gguf_file, "cohere-transcribe-q4_k.gguf");
    }

    #[test]
    fn rejects_missing_key() {
        assert!(parse_pin("crispasr_version=0.4.6\n").is_err());
    }

    #[test]
    fn rejects_non_hex_sha() {
        let bad = SAMPLE.replace(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "zzzz",
        );
        assert!(parse_pin(&bad).is_err());
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::pin`
Expected: FAIL — `cannot find function parse_pin` / `cannot find type CrispasrPin`.

- [ ] **Step 4: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/pin.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrispasrPin {
    pub crispasr_version: String,
    pub binary_sha256: String,
    pub gguf_repo: String,
    pub gguf_revision: String,
    pub gguf_file: String,
    pub gguf_sha256: String,
}

pub const PIN_TEXT: &str = include_str!("../../../crispasr-version.txt");

pub fn load_pin() -> Result<CrispasrPin, String> {
    parse_pin(PIN_TEXT)
}

pub fn parse_pin(text: &str) -> Result<CrispasrPin, String> {
    let mut version = None;
    let mut binary_sha = None;
    let mut repo = None;
    let mut revision = None;
    let mut file = None;
    let mut gguf_sha = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("crispasr-version.txt: missing '=' in line: {line}"))?;
        let value = value.trim().to_string();
        match key.trim() {
            "crispasr_version" => version = Some(value),
            "binary_sha256" => binary_sha = Some(value),
            "gguf_repo" => repo = Some(value),
            "gguf_revision" => revision = Some(value),
            "gguf_file" => file = Some(value),
            "gguf_sha256" => gguf_sha = Some(value),
            other => return Err(format!("crispasr-version.txt: unknown key: {other}")),
        }
    }

    let require = |field: Option<String>, name: &str| {
        field.ok_or_else(|| format!("crispasr-version.txt: missing key: {name}"))
    };
    let binary_sha256 = require(binary_sha, "binary_sha256")?;
    let gguf_sha256 = require(gguf_sha, "gguf_sha256")?;
    if !is_sha256(&binary_sha256) {
        return Err("crispasr-version.txt: binary_sha256 must be 64 hex chars".into());
    }
    if !is_sha256(&gguf_sha256) {
        return Err("crispasr-version.txt: gguf_sha256 must be 64 hex chars".into());
    }

    Ok(CrispasrPin {
        crispasr_version: require(version, "crispasr_version")?,
        binary_sha256,
        gguf_repo: require(repo, "gguf_repo")?,
        gguf_revision: require(revision, "gguf_revision")?,
        gguf_file: require(file, "gguf_file")?,
        gguf_sha256,
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::pin`
Expected: PASS. (Requires `desktop/crispasr-version.txt` to exist from Step 1; `include_str!` reads it at compile time.)

- [ ] **Step 6: Commit**

```bash
git add desktop/crispasr-version.txt desktop/src-tauri/src/stt/pin.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): pin crispasr version + binary/gguf SHA-256 with validating parser"
```

### Task 4: Model cache — dir resolution + SHA-256 verify

**Files:**
- Create: `desktop/src-tauri/src/stt/model.rs`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

Resolves the models dir (`YAP_MODELS_DIR` → `%LOCALAPPDATA%/Yap/models/`) and provides fail-closed SHA-256 verification (§8). A missing file → `MODEL_MISSING`; a hash mismatch → `MODEL_CORRUPT`.

- [ ] **Step 1: Write the failing test**

Add `pub mod model;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/model.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_dir_prefers_override() {
        let dir = models_dir_from(|key| match key {
            "YAP_MODELS_DIR" => Some("D:/custom".into()),
            _ => None,
        });
        assert_eq!(dir, std::path::PathBuf::from("D:/custom"));
    }

    #[test]
    fn models_dir_falls_back_to_localappdata() {
        let dir = models_dir_from(|key| match key {
            "LOCALAPPDATA" => Some("C:/Users/me/AppData/Local".into()),
            _ => None,
        });
        assert_eq!(
            dir,
            std::path::PathBuf::from("C:/Users/me/AppData/Local").join("Yap").join("models")
        );
    }

    #[test]
    fn verify_sha256_matches_and_mismatches() {
        let path = std::env::temp_dir().join(format!("yap-sha-{}.bin", std::process::id()));
        std::fs::write(&path, b"hello").unwrap();
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_sha256(&path, expected).is_ok());
        assert_eq!(verify_sha256(&path, &"0".repeat(64)).unwrap_err(), SttError::ModelCorrupt);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn verify_sha256_missing_file_is_model_missing() {
        let path = std::env::temp_dir().join("yap-absent-3f9c1a.bin");
        assert_eq!(verify_sha256(&path, &"0".repeat(64)).unwrap_err(), SttError::ModelMissing);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::model`
Expected: FAIL — `cannot find function models_dir_from` / `verify_sha256`.

- [ ] **Step 3: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/model.rs`:

```rust
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::stt::error::SttError;

pub fn models_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = env("YAP_MODELS_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(local) = env("LOCALAPPDATA") {
        return PathBuf::from(local).join("Yap").join("models");
    }
    PathBuf::from(".").join("models")
}

pub fn models_dir() -> PathBuf {
    models_dir_from(|key| std::env::var(key).ok())
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
}

pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), SttError> {
    let actual = sha256_file(path).map_err(|_| SttError::ModelMissing)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(SttError::ModelCorrupt)
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::model`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/model.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): models dir resolution + fail-closed SHA-256 verification"
```

### Task 5: Pinned GGUF download (fail-closed)

**Files:**
- Modify: `desktop/src-tauri/src/stt/model.rs`

On first use, download the exact pinned HF revision over HTTPS, then verify before use; a mismatch deletes the file and fails closed (§8). The download is factored behind a closure so the orchestration is unit-testable without network.

- [ ] **Step 1: Write the failing test**

Append this test module to `desktop/src-tauri/src/stt/model.rs` (below the existing `mod tests`... instead, extend the existing `tests` module by adding these functions inside it):

```rust
    #[test]
    fn hf_resolve_url_is_pinned_by_revision() {
        assert_eq!(
            hf_resolve_url("owner/repo", "abc123", "model.gguf"),
            "https://huggingface.co/owner/repo/resolve/abc123/model.gguf"
        );
    }

    fn sample_pin(gguf_sha256: &str) -> crate::stt::pin::CrispasrPin {
        crate::stt::pin::CrispasrPin {
            crispasr_version: "0.4.6".into(),
            binary_sha256: "a".repeat(64),
            gguf_repo: "owner/repo".into(),
            gguf_revision: "rev".into(),
            gguf_file: "m.gguf".into(),
            gguf_sha256: gguf_sha256.into(),
        }
    }

    #[test]
    fn ensure_model_downloads_then_verifies() {
        let dir = std::env::temp_dir().join(format!("yap-dl-ok-{}", std::process::id()));
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let pin = sample_pin(hello);
        let dest = ensure_model_at(&dir, &pin, |_url, path| {
            std::fs::write(path, b"hello").map_err(|_| SttError::ModelMissing)
        })
        .unwrap();
        assert!(dest.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_model_rejects_corrupt_download() {
        let dir = std::env::temp_dir().join(format!("yap-dl-bad-{}", std::process::id()));
        let pin = sample_pin(&"0".repeat(64));
        let err = ensure_model_at(&dir, &pin, |_url, path| {
            std::fs::write(path, b"tampered").map_err(|_| SttError::ModelMissing)
        })
        .unwrap_err();
        assert_eq!(err, SttError::ModelCorrupt);
        assert!(!dir.join("m.gguf").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_model_uses_valid_cache_without_downloading() {
        let dir = std::env::temp_dir().join(format!("yap-dl-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.gguf"), b"hello").unwrap();
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let pin = sample_pin(hello);
        let dest = ensure_model_at(&dir, &pin, |_url, _path| {
            panic!("download must not run when a valid cache exists")
        })
        .unwrap();
        assert!(dest.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::model`
Expected: FAIL — `cannot find function hf_resolve_url` / `ensure_model_at`.

- [ ] **Step 3: Write the minimal implementation**

Add these functions to `desktop/src-tauri/src/stt/model.rs` (above the test module). Add `use crate::stt::pin::CrispasrPin;` to the existing imports at the top of the file:

```rust
pub fn hf_resolve_url(repo: &str, revision: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/{revision}/{file}")
}

pub fn ensure_model_at<D>(dir: &Path, pin: &CrispasrPin, download: D) -> Result<PathBuf, SttError>
where
    D: Fn(&str, &Path) -> Result<(), SttError>,
{
    let dest = dir.join(&pin.gguf_file);
    if dest.exists() {
        verify_sha256(&dest, &pin.gguf_sha256)?;
        return Ok(dest);
    }
    std::fs::create_dir_all(dir).map_err(|_| SttError::ModelMissing)?;
    let url = hf_resolve_url(&pin.gguf_repo, &pin.gguf_revision, &pin.gguf_file);
    download(&url, &dest)?;
    match verify_sha256(&dest, &pin.gguf_sha256) {
        Ok(()) => Ok(dest),
        Err(err) => {
            let _ = std::fs::remove_file(&dest);
            Err(err)
        }
    }
}

pub fn download_file(url: &str, dest: &Path) -> Result<(), SttError> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|_| SttError::ModelMissing)?;
    let mut response = client.get(url).send().map_err(|_| SttError::ModelMissing)?;
    if !response.status().is_success() {
        return Err(SttError::ModelMissing);
    }
    let tmp = dest.with_extension("part");
    let mut file = std::fs::File::create(&tmp).map_err(|_| SttError::ModelMissing)?;
    std::io::copy(&mut response, &mut file).map_err(|_| SttError::ModelMissing)?;
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|_| SttError::ModelMissing)?;
    Ok(())
}

pub fn ensure_model() -> Result<PathBuf, SttError> {
    let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;
    ensure_model_at(&models_dir(), &pin, download_file)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::model`
Expected: PASS (four model tests plus the two from Task 4).

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/model.rs
git commit -m "feat(stt): pinned GGUF download with fail-closed verify"
```

### Task 6: `SttBackend` trait + `select_backend`

**Files:**
- Create: `desktop/src-tauri/src/stt/backend.rs`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

The uniform batch API (§5) plus the `YAP_STT_BACKEND` selection logic (§12) — the pure function that the spec's §15 "backend dispatch" test targets. `transcribe_batch` has a per-file default; `PythonBackend` overrides it (Task 7) to keep a single model-load.

- [ ] **Step 1: Write the failing test**

Add `pub mod backend;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/backend.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct FakeBackend;
    impl SttBackend for FakeBackend {
        fn transcribe(&self, _audio: &Path, _language: &str) -> Result<String, crate::stt::error::SttError> {
            Ok("hi".to_string())
        }
    }

    #[test]
    fn selects_backend_from_env_value() {
        assert_eq!(select_backend(Some("crispasr")), BackendChoice::Crispasr);
        assert_eq!(select_backend(Some("python")), BackendChoice::Python);
        assert_eq!(select_backend(None), BackendChoice::PreferCrispasr);
        assert_eq!(select_backend(Some("bogus")), BackendChoice::PreferCrispasr);
    }

    #[test]
    fn transcribe_batch_defaults_to_per_file_loop() {
        let backend = FakeBackend;
        let files = vec![PathBuf::from("a.wav"), PathBuf::from("b.wav")];
        let out = backend.transcribe_batch(&files, "en");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].as_ref().unwrap(), "hi");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::backend`
Expected: FAIL — `cannot find type SttBackend` / `select_backend` / `BackendChoice`.

- [ ] **Step 3: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/backend.rs`:

```rust
use std::path::{Path, PathBuf};

use crate::stt::error::SttError;

pub trait SttBackend {
    fn transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError>;

    fn transcribe_batch(&self, files: &[PathBuf], language: &str) -> Vec<Result<String, SttError>> {
        files.iter().map(|file| self.transcribe(file, language)).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    Crispasr,
    Python,
    PreferCrispasr,
}

pub fn select_backend(value: Option<&str>) -> BackendChoice {
    match value {
        Some("crispasr") => BackendChoice::Crispasr,
        Some("python") => BackendChoice::Python,
        Some(_) | None => BackendChoice::PreferCrispasr,
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::backend`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/backend.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): SttBackend trait + YAP_STT_BACKEND selection"
```

### Task 7: `PythonBackend` (verbatim fallback)

**Files:**
- Create: `desktop/src-tauri/src/stt/python.rs`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

Wraps today's `transcribe.py` invocation (§5). Overrides `transcribe_batch` to spawn Python **once** for the whole batch (single model-load, no Phase-0 regression); writes to a temp out-dir and returns text so the dispatcher owns the canonical sibling `<stem>.txt`. Failures map to `SttError` with engine-agnostic toasts; full detail goes to the log.

- [ ] **Step 1: Write the failing test**

Add `pub mod python;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/python.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command_targets_venv_python_and_script() {
        let backend = PythonBackend::new(PathBuf::from("C:/proj"));
        let files = [PathBuf::from("C:/clips/a.wav")];
        let command = backend.build_command(&files, "en", &PathBuf::from("C:/tmp/out"));
        let program = command.get_program().to_string_lossy().to_string();
        assert!(program.ends_with("python.exe"));
        let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a.ends_with("transcribe.py")));
        assert!(args.contains(&"--language".to_string()));
        assert!(args.contains(&"en".to_string()));
        assert!(args.contains(&"--out-dir".to_string()));
    }

    #[test]
    fn classify_python_failure_maps_known_causes() {
        assert_eq!(classify_python_failure("CUDA out of memory"), SttError::Oom);
        assert_eq!(classify_python_failure("Repo is gated, requires approval"), SttError::ModelMissing);
        assert_eq!(classify_python_failure("soundfile failed to open the file"), SttError::AudioDecode);
        assert_eq!(classify_python_failure("some unexpected traceback"), SttError::SidecarCrash);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::python`
Expected: FAIL — `cannot find type PythonBackend` / `classify_python_failure`.

- [ ] **Step 3: Add the shared process/log helpers to the module root**

Add to the top of `desktop/src-tauri/src/stt/mod.rs` (imports first, per repo rules), keeping the existing `pub mod` lines below:

```rust
use std::io::Write;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub(crate) fn hide_child_console(command: &mut std::process::Command) {
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
pub(crate) fn hide_child_console(_command: &mut std::process::Command) {}

pub(crate) fn log_stt(message: &str) {
    let path = stt_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let _ = writeln!(file, "{stamp} {message}");
    }
}

fn stt_log_path() -> std::path::PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return std::path::PathBuf::from(local).join("Yap").join("logs").join("crispasr.log");
    }
    std::path::PathBuf::from("crispasr.log")
}
```

- [ ] **Step 4: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/python.rs`:

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::stt::backend::SttBackend;
use crate::stt::error::SttError;

pub struct PythonBackend {
    root: PathBuf,
}

impl PythonBackend {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn python(&self) -> PathBuf {
        self.root.join(".venv").join("Scripts").join("python.exe")
    }

    fn script(&self) -> PathBuf {
        self.root.join("transcribe.py")
    }

    pub fn build_command(&self, files: &[PathBuf], language: &str, out_dir: &Path) -> Command {
        let mut command = Command::new(self.python());
        command.current_dir(&self.root).arg(self.script());
        for file in files {
            command.arg(file);
        }
        command.arg("--language").arg(language).arg("--out-dir").arg(out_dir);
        crate::stt::hide_child_console(&mut command);
        command
    }
}

impl SttBackend for PythonBackend {
    fn transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError> {
        let files = [audio.to_path_buf()];
        self.transcribe_batch(&files, language)
            .into_iter()
            .next()
            .unwrap_or(Err(SttError::AudioDecode))
    }

    fn transcribe_batch(&self, files: &[PathBuf], language: &str) -> Vec<Result<String, SttError>> {
        if files.is_empty() {
            return Vec::new();
        }
        if !self.python().exists() || !self.script().exists() {
            return errors_for(files.len(), SttError::SidecarUnreachable);
        }
        let out_dir = match temp_out_dir() {
            Ok(dir) => dir,
            Err(_) => return errors_for(files.len(), SttError::SidecarCrash),
        };
        let output = self.build_command(files, language, &out_dir).output();
        let result = match output {
            Ok(output) if output.status.success() => {
                let produced: Vec<String> = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect();
                files
                    .iter()
                    .enumerate()
                    .map(|(index, _)| match produced.get(index) {
                        Some(path) => std::fs::read_to_string(path)
                            .map(|text| text.trim().to_string())
                            .map_err(|_| SttError::AudioDecode),
                        None => Err(SttError::AudioDecode),
                    })
                    .collect()
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                crate::stt::log_stt(&format!("python backend failed: {}", stderr.trim()));
                errors_for(files.len(), classify_python_failure(&stderr))
            }
            Err(err) => {
                crate::stt::log_stt(&format!("python backend spawn error: {err}"));
                errors_for(files.len(), SttError::SidecarUnreachable)
            }
        };
        let _ = std::fs::remove_dir_all(&out_dir);
        result
    }
}

fn errors_for(count: usize, error: SttError) -> Vec<Result<String, SttError>> {
    (0..count).map(|_| Err(error)).collect()
}

fn temp_out_dir() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("yap-stt-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn classify_python_failure(stderr: &str) -> SttError {
    let lower = stderr.to_lowercase();
    if lower.contains("out of memory") || lower.contains("memoryerror") {
        SttError::Oom
    } else if lower.contains("gated") || lower.contains("requires approval") || lower.contains("access denied") {
        SttError::ModelMissing
    } else if lower.contains("soundfile") || lower.contains("load_audio") || lower.contains("ffmpeg") {
        SttError::AudioDecode
    } else {
        SttError::SidecarCrash
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::python`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add desktop/src-tauri/src/stt/python.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): PythonBackend wrapping transcribe.py as batch-preserving fallback"
```

### Task 8: Sidecar pure helpers (port, env, args, health, idle, resolve)

**Files:**
- Create: `desktop/src-tauri/src/stt/sidecar.rs`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

The pure, unit-testable pieces of the lifecycle (§7, §9, §10): port probe (8765→8775), scrubbed-env allowlist, launch args (loopback-only), health gate, idle threshold, binary resolution. The stateful manager wraps these in Task 10.

- [ ] **Step 1: Write the failing test**

Add `pub mod sidecar;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/sidecar.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_free_port_returns_first_matching() {
        assert_eq!(first_free_port(8765..=8775, |p| p == 8770), Some(8770));
        assert_eq!(first_free_port(8765..=8767, |_| false), None);
    }

    #[test]
    fn sidecar_env_drops_secrets_keeps_allowlist() {
        let source = vec![
            ("PATH".to_string(), "C:/bin".to_string()),
            ("HF_TOKEN".to_string(), "secret".to_string()),
            ("GITHUB_TOKEN".to_string(), "secret".to_string()),
            ("SystemRoot".to_string(), "C:/Windows".to_string()),
        ];
        let scrubbed = sidecar_env(source);
        assert!(scrubbed.iter().any(|(k, _)| k == "PATH"));
        assert!(scrubbed.iter().any(|(k, _)| k == "SystemRoot"));
        assert!(!scrubbed.iter().any(|(k, _)| k.eq_ignore_ascii_case("HF_TOKEN")));
        assert!(!scrubbed.iter().any(|(k, _)| k.eq_ignore_ascii_case("GITHUB_TOKEN")));
    }

    #[test]
    fn launch_args_bind_loopback_only() {
        let args = build_launch_args(std::path::Path::new("C:/models/m.gguf"), 8765);
        assert_eq!(args[0], "--server");
        let host = args.iter().position(|a| a == "--host").unwrap();
        assert_eq!(args[host + 1], "127.0.0.1");
        let port = args.iter().position(|a| a == "--port").unwrap();
        assert_eq!(args[port + 1], "8765");
    }

    #[test]
    fn health_ready_requires_ok_and_cohere() {
        assert!(health_is_ready(r#"{"status":"ok","backend":"cohere"}"#));
        assert!(!health_is_ready(r#"{"status":"ok","backend":"whisper"}"#));
        assert!(!health_is_ready(r#"{"status":"loading","backend":"cohere"}"#));
        assert!(!health_is_ready("not json"));
    }

    #[test]
    fn should_unload_after_threshold() {
        assert!(should_unload(std::time::Duration::from_secs(601), IDLE_UNLOAD));
        assert!(!should_unload(std::time::Duration::from_secs(10), IDLE_UNLOAD));
    }

    #[test]
    fn resolve_binary_missing_dev_override_is_unreachable() {
        let err = resolve_binary(|_| Some("C:/definitely/not/here.exe".into()), std::path::Path::new("C:/app"));
        assert_eq!(err.unwrap_err(), SttError::SidecarUnreachable);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::sidecar`
Expected: FAIL — unresolved names (`first_free_port`, `sidecar_env`, …).

- [ ] **Step 3: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/sidecar.rs`:

```rust
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::stt::error::SttError;

pub const PORT_RANGE: RangeInclusive<u16> = 8765..=8775;
pub const READY_BUDGET: Duration = Duration::from_secs(10);
pub const IDLE_UNLOAD: Duration = Duration::from_secs(600);
const HOST: &str = "127.0.0.1";
const ENV_ALLOWLIST: [&str; 4] = ["PATH", "SYSTEMROOT", "TEMP", "TMP"];

pub fn first_free_port(range: RangeInclusive<u16>, mut is_free: impl FnMut(u16) -> bool) -> Option<u16> {
    range.into_iter().find(|port| is_free(*port))
}

pub fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind((HOST, port)).is_ok()
}

pub fn probe_port() -> Option<u16> {
    first_free_port(PORT_RANGE, port_is_free)
}

pub fn sidecar_env<I>(source: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    source
        .into_iter()
        .filter(|(key, _)| ENV_ALLOWLIST.iter().any(|allowed| key.eq_ignore_ascii_case(allowed)))
        .collect()
}

pub fn build_launch_args(gguf: &Path, port: u16) -> Vec<String> {
    vec![
        "--server".to_string(),
        "-m".to_string(),
        gguf.to_string_lossy().to_string(),
        "--host".to_string(),
        HOST.to_string(),
        "--port".to_string(),
        port.to_string(),
    ]
}

pub fn health_is_ready(json: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(value) => {
            value.get("status").and_then(serde_json::Value::as_str) == Some("ok")
                && value.get("backend").and_then(serde_json::Value::as_str) == Some("cohere")
        }
        Err(_) => false,
    }
}

pub fn should_unload(idle: Duration, threshold: Duration) -> bool {
    idle >= threshold
}

pub fn sidecar_binary_path(exe_dir: &Path) -> PathBuf {
    let name = if cfg!(windows) { "crispasr.exe" } else { "crispasr" };
    exe_dir.join(name)
}

pub fn resolve_binary<F>(env: F, exe_dir: &Path) -> Result<PathBuf, SttError>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dev) = env("YAP_CRISPASR_BIN") {
        let path = PathBuf::from(dev);
        return if path.exists() { Ok(path) } else { Err(SttError::SidecarUnreachable) };
    }
    let bundled = sidecar_binary_path(exe_dir);
    if bundled.exists() {
        Ok(bundled)
    } else {
        Err(SttError::SidecarUnreachable)
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::sidecar`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/sidecar.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): sidecar pure helpers (port/env/args/health/idle/resolve)"
```

### Task 9: CrispASR HTTP parsing + input validation

**Files:**
- Create: `desktop/src-tauri/src/stt/crispasr.rs`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

The trust-boundary logic (§6.2, §10.3): tolerant response parsing (read `text`, ignore unknown fields), HTTP status → `SttError` classification, and input validation (path is a real file, bounded size). All pure — the networked client comes in Task 11.

- [ ] **Step 1: Write the failing test**

Add `pub mod crispasr;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/crispasr.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_text_and_ignores_unknown_fields() {
        let body = r#"{"text":"hello world","segments":[{"start":0.0}],"backend":"cohere","extra":42}"#;
        assert_eq!(parse_transcription_json(body).unwrap(), "hello world");
    }

    #[test]
    fn parse_accepts_empty_text() {
        assert_eq!(parse_transcription_json(r#"{"text":""}"#).unwrap(), "");
    }

    #[test]
    fn parse_rejects_missing_text_and_bad_json() {
        assert_eq!(parse_transcription_json(r#"{"segments":[]}"#).unwrap_err(), SttError::SidecarCrash);
        assert_eq!(parse_transcription_json("not json").unwrap_err(), SttError::SidecarCrash);
    }

    #[test]
    fn classify_response_maps_status_and_body() {
        assert_eq!(classify_response(408, ""), SttError::Timeout);
        assert_eq!(classify_response(400, "unsupported language code"), SttError::BadLang);
        assert_eq!(classify_response(500, "ggml out of memory"), SttError::Oom);
        assert_eq!(classify_response(500, "failed to decode audio"), SttError::AudioDecode);
        assert_eq!(classify_response(500, "panic"), SttError::SidecarCrash);
    }

    #[test]
    fn check_audio_size_rejects_empty_and_oversized() {
        assert!(check_audio_size(1, MAX_AUDIO_BYTES).is_ok());
        assert_eq!(check_audio_size(0, MAX_AUDIO_BYTES).unwrap_err(), SttError::AudioDecode);
        assert_eq!(check_audio_size(MAX_AUDIO_BYTES + 1, MAX_AUDIO_BYTES).unwrap_err(), SttError::AudioDecode);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::crispasr`
Expected: FAIL — unresolved names (`parse_transcription_json`, `classify_response`, …).

- [ ] **Step 3: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/crispasr.rs`:

```rust
use std::path::Path;

use crate::stt::error::SttError;

pub const MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn parse_transcription_json(body: &str) -> Result<String, SttError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| SttError::SidecarCrash)?;
    match value.get("text").and_then(serde_json::Value::as_str) {
        Some(text) => Ok(text.to_string()),
        None => Err(SttError::SidecarCrash),
    }
}

pub fn classify_response(status: u16, body: &str) -> SttError {
    let lower = body.to_lowercase();
    if status == 408 {
        SttError::Timeout
    } else if lower.contains("language") {
        SttError::BadLang
    } else if lower.contains("out of memory") || lower.contains("oom") {
        SttError::Oom
    } else if lower.contains("decode") || lower.contains("audio") {
        SttError::AudioDecode
    } else {
        SttError::SidecarCrash
    }
}

pub fn check_audio_size(len: u64, max: u64) -> Result<(), SttError> {
    if len == 0 || len > max {
        Err(SttError::AudioDecode)
    } else {
        Ok(())
    }
}

pub fn validate_audio_input(path: &Path) -> Result<(), SttError> {
    let metadata = std::fs::metadata(path).map_err(|_| SttError::AudioDecode)?;
    if !metadata.is_file() {
        return Err(SttError::AudioDecode);
    }
    check_audio_size(metadata.len(), MAX_AUDIO_BYTES)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::crispasr`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/crispasr.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): tolerant CrispASR response parsing + input validation"
```

### Task 10: `CrispasrSidecar` manager (spawn, ready-gate, restart, idle, shutdown)

**Files:**
- Modify: `desktop/src-tauri/src/stt/sidecar.rs`

The stateful lifecycle (§7): lazy spawn with a scrubbed env, `/health` ready-gate (10 s → `SIDECAR_UNREACHABLE`), binary SHA-256 re-verify before spawn, restart, idle-unload, and kill. Unit tests cover the state machine's pure edges; spawn/ready-gate/restart are exercised by the gated integration test (Task 16) since they need the real binary + model.

- [ ] **Step 1: Write the failing test**

Append these tests to the existing `mod tests` in `desktop/src-tauri/src/stt/sidecar.rs`:

```rust
    #[test]
    fn new_sidecar_is_not_running_and_has_no_url() {
        let mut sidecar = CrispasrSidecar::new();
        assert!(!sidecar.is_running());
        assert!(sidecar.base_url().is_none());
        sidecar.shutdown(); // no panic when there is no child
    }

    #[test]
    fn base_url_uses_loopback_and_port() {
        let mut sidecar = CrispasrSidecar::new();
        sidecar.port = Some(8770);
        assert_eq!(sidecar.base_url().unwrap(), "http://127.0.0.1:8770");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::sidecar`
Expected: FAIL — `cannot find type CrispasrSidecar`.

- [ ] **Step 3: Write the minimal implementation**

Extend the top-of-file imports in `desktop/src-tauri/src/stt/sidecar.rs` to:

```rust
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::stt::error::SttError;
```

Then add the manager (above the test module):

```rust
pub struct CrispasrSidecar {
    child: Option<Child>,
    port: Option<u16>,
    last_used: Instant,
}

impl CrispasrSidecar {
    pub fn new() -> Self {
        Self { child: None, port: None, last_used: Instant::now() }
    }

    pub fn base_url(&self) -> Option<String> {
        self.port.map(|port| format!("http://{HOST}:{port}"))
    }

    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) => false,
                Err(_) => false,
            },
            None => false,
        }
    }

    pub fn mark_used(&mut self) {
        self.last_used = Instant::now();
    }

    pub fn ensure_ready(&mut self) -> Result<String, SttError> {
        if self.is_running() {
            if let Some(url) = self.base_url() {
                self.mark_used();
                return Ok(url);
            }
        }
        self.shutdown();

        let binary = resolve_binary(|key| std::env::var(key).ok(), &current_exe_dir())?;
        let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;
        if crate::stt::model::verify_sha256(&binary, &pin.binary_sha256).is_err() {
            crate::stt::log_stt("crispasr binary failed SHA-256 verification; refusing to spawn");
            return Err(SttError::SidecarUnreachable);
        }
        let model = crate::stt::model::ensure_model()?;
        let port = probe_port().ok_or(SttError::SidecarUnreachable)?;

        let child = spawn_child(&binary, &model, port)?;
        self.child = Some(child);
        self.port = Some(port);

        let url = self.base_url().ok_or(SttError::SidecarUnreachable)?;
        if wait_ready(&url) {
            self.mark_used();
            crate::stt::log_stt(&format!("crispasr sidecar ready on {url}"));
            Ok(url)
        } else {
            crate::stt::log_stt("crispasr sidecar failed the 10s ready-gate");
            self.shutdown();
            Err(SttError::SidecarUnreachable)
        }
    }

    pub fn restart(&mut self) -> Result<String, SttError> {
        crate::stt::log_stt("crispasr sidecar restart-once");
        self.shutdown();
        self.ensure_ready()
    }

    pub fn unload_if_idle(&mut self) {
        if self.is_running() && should_unload(self.last_used.elapsed(), IDLE_UNLOAD) {
            crate::stt::log_stt("crispasr sidecar idle-unload after 10min");
            self.shutdown();
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.port = None;
    }
}

impl Default for CrispasrSidecar {
    fn default() -> Self {
        Self::new()
    }
}

fn current_exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn spawn_child(binary: &Path, model: &Path, port: u16) -> Result<Child, SttError> {
    let mut command = Command::new(binary);
    command.args(build_launch_args(model, port));
    command.env_clear();
    command.envs(sidecar_env(std::env::vars()));
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    crate::stt::hide_child_console(&mut command);
    command.spawn().map_err(|_| SttError::SidecarUnreachable)
}

fn wait_ready(base_url: &str) -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let deadline = Instant::now() + READY_BUDGET;
    while Instant::now() < deadline {
        if let Ok(response) = client.get(format!("{base_url}/health")).send() {
            if response.status().is_success() {
                if let Ok(body) = response.text() {
                    if health_is_ready(&body) {
                        return true;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::sidecar`
Expected: PASS (the eight sidecar tests). Spawn/ready-gate are integration-verified in Task 16.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/sidecar.rs
git commit -m "feat(stt): CrispasrSidecar manager (spawn/ready-gate/restart/idle/kill)"
```

### Task 11: `CrispasrBackend` (HTTP client + one-in-flight + restart/retry)

**Files:**
- Modify: `desktop/src-tauri/src/stt/crispasr.rs`

Ties parsing (Task 9) to the manager (Task 10): one-request-in-flight (`BUSY` via `try_lock`), restart-once + retry-file-once → `SIDECAR_CRASH` (§7). The retry orchestration is a pure function tested with fake closures; the real `POST /v1/audio/transcriptions` runs in Task 16.

- [ ] **Step 1: Write the failing test**

In `desktop/src-tauri/src/stt/crispasr.rs`, add these imports at the top of the existing `mod tests` block (below `use super::*;`):

```rust
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};

    use crate::stt::sidecar::CrispasrSidecar;
```

Then append these tests inside `mod tests`:

```rust
    #[test]
    fn retry_succeeds_on_first_post() {
        let calls = Cell::new(0);
        let out = run_with_retry(
            || Ok("url".to_string()),
            || Ok("url2".to_string()),
            |_url| {
                calls.set(calls.get() + 1);
                Ok("hi".to_string())
            },
        );
        assert_eq!(out.unwrap(), "hi");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn retry_restarts_then_succeeds() {
        let calls = Cell::new(0);
        let out = run_with_retry(
            || Ok("url".to_string()),
            || Ok("url2".to_string()),
            |_url| {
                let n = calls.get();
                calls.set(n + 1);
                if n == 0 {
                    Err(SttError::SidecarCrash)
                } else {
                    Ok("recovered".to_string())
                }
            },
        );
        assert_eq!(out.unwrap(), "recovered");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn retry_gives_up_after_second_failure() {
        let out = run_with_retry(
            || Ok("url".to_string()),
            || Ok("url2".to_string()),
            |_url| Err(SttError::SidecarUnreachable),
        );
        assert_eq!(out.unwrap_err(), SttError::SidecarCrash);
    }

    #[test]
    fn retry_propagates_non_sidecar_errors_without_restart() {
        let restarted = Cell::new(false);
        let out = run_with_retry(
            || Ok("url".to_string()),
            || {
                restarted.set(true);
                Ok("url2".to_string())
            },
            |_url| Err(SttError::AudioDecode),
        );
        assert_eq!(out.unwrap_err(), SttError::AudioDecode);
        assert!(!restarted.get());
    }

    #[test]
    fn transcribe_returns_busy_when_a_request_is_in_flight() {
        let backend = CrispasrBackend::new(Arc::new(Mutex::new(CrispasrSidecar::new())));
        let _held = backend.inflight.lock().unwrap();
        let err = backend.transcribe(Path::new("C:/clips/a.wav"), "en").unwrap_err();
        assert_eq!(err, SttError::Busy);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::crispasr`
Expected: FAIL — `cannot find function run_with_retry` / `cannot find type CrispasrBackend`.

- [ ] **Step 3: Write the minimal implementation**

Replace the import block at the top of `desktop/src-tauri/src/stt/crispasr.rs` with:

```rust
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::stt::backend::SttBackend;
use crate::stt::error::SttError;
use crate::stt::sidecar::CrispasrSidecar;
```

Then add (above the test module, below the Task 9 pure functions):

```rust
pub struct CrispasrBackend {
    sidecar: Arc<Mutex<CrispasrSidecar>>,
    inflight: Arc<Mutex<()>>,
}

impl CrispasrBackend {
    pub fn new(sidecar: Arc<Mutex<CrispasrSidecar>>) -> Self {
        Self { sidecar, inflight: Arc::new(Mutex::new(())) }
    }
}

impl SttBackend for CrispasrBackend {
    fn transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError> {
        let _inflight = self.inflight.try_lock().map_err(|_| SttError::Busy)?;
        validate_audio_input(audio)?;

        let sidecar = Arc::clone(&self.sidecar);
        let ensure = || -> Result<String, SttError> {
            sidecar.lock().map_err(|_| SttError::SidecarCrash)?.ensure_ready()
        };
        let restart = || -> Result<String, SttError> {
            sidecar.lock().map_err(|_| SttError::SidecarCrash)?.restart()
        };

        let result = run_with_retry(ensure, restart, |url| post_transcription(url, audio, language));
        if result.is_ok() {
            if let Ok(mut guard) = self.sidecar.lock() {
                guard.mark_used();
            }
        }
        result
    }
}

fn is_sidecar_failure(error: SttError) -> bool {
    match error {
        SttError::SidecarCrash | SttError::SidecarUnreachable | SttError::Timeout => true,
        SttError::ModelMissing
        | SttError::ModelCorrupt
        | SttError::BadLang
        | SttError::Oom
        | SttError::AudioDecode
        | SttError::Busy => false,
    }
}

fn run_with_retry<E, R, P>(ensure: E, restart: R, mut post: P) -> Result<String, SttError>
where
    E: Fn() -> Result<String, SttError>,
    R: Fn() -> Result<String, SttError>,
    P: FnMut(&str) -> Result<String, SttError>,
{
    let url = ensure()?;
    match post(&url) {
        Ok(text) => Ok(text),
        Err(error) if is_sidecar_failure(error) => {
            let url = restart()?;
            match post(&url) {
                Ok(text) => Ok(text),
                Err(_) => Err(SttError::SidecarCrash),
            }
        }
        Err(error) => Err(error),
    }
}

fn post_transcription(base_url: &str, audio: &Path, language: &str) -> Result<String, SttError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|_| SttError::SidecarUnreachable)?;
    let form = reqwest::blocking::multipart::Form::new()
        .file("file", audio)
        .map_err(|_| SttError::AudioDecode)?
        .text("language", language.to_string());
    let response = client
        .post(format!("{base_url}/v1/audio/transcriptions"))
        .multipart(form)
        .send()
        .map_err(|err| if err.is_timeout() { SttError::Timeout } else { SttError::SidecarCrash })?;
    let status = response.status();
    let body = response.text().map_err(|_| SttError::SidecarCrash)?;
    if status.is_success() {
        parse_transcription_json(&body)
    } else {
        Err(classify_response(status.as_u16(), &body))
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::crispasr`
Expected: PASS (Task 9 tests plus the five new ones).

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/crispasr.rs
git commit -m "feat(stt): CrispasrBackend with one-in-flight + restart/retry"
```

### Task 12: Parity helpers — WER + `verbose_json` timestamp probe

**Files:**
- Create: `desktop/src-tauri/src/stt/parity.rs`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

Pure helpers that feed the trust-bar parity test (§12.1, §15): a word-level WER and a tolerant `verbose_json` timestamp detector (confirms segment/word timing is present, for Phase 7).

- [ ] **Step 1: Write the failing test**

Add `pub mod parity;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/parity.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wer_zero_for_identical() {
        assert_eq!(word_error_rate("the quick brown fox", "the quick brown fox"), 0.0);
    }

    #[test]
    fn wer_counts_one_substitution() {
        assert!((word_error_rate("the quick brown fox", "the quick green fox") - 0.25).abs() < 1e-9);
    }

    #[test]
    fn wer_is_case_insensitive() {
        assert_eq!(word_error_rate("Hello World", "hello world"), 0.0);
    }

    #[test]
    fn wer_handles_empty_reference() {
        assert_eq!(word_error_rate("", ""), 0.0);
        assert_eq!(word_error_rate("", "extra"), 1.0);
    }

    #[test]
    fn verbose_json_detects_segment_and_word_timing() {
        assert!(parse_verbose_json_has_timestamps(
            r#"{"text":"hi","segments":[{"start":0.0,"end":1.2,"text":"hi"}]}"#
        ));
        assert!(parse_verbose_json_has_timestamps(
            r#"{"text":"hi","words":[{"word":"hi","start":0.0,"end":0.4}]}"#
        ));
    }

    #[test]
    fn verbose_json_false_without_timing() {
        assert!(!parse_verbose_json_has_timestamps(r#"{"text":"hi"}"#));
        assert!(!parse_verbose_json_has_timestamps("not json"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::parity`
Expected: FAIL — `cannot find function word_error_rate` / `parse_verbose_json_has_timestamps`.

- [ ] **Step 3: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/parity.rs`:

```rust
pub fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let reference: Vec<&str> = reference.split_whitespace().collect();
    let hypothesis: Vec<&str> = hypothesis.split_whitespace().collect();
    if reference.is_empty() {
        return if hypothesis.is_empty() { 0.0 } else { 1.0 };
    }
    edit_distance(&reference, &hypothesis) as f64 / reference.len() as f64
}

fn edit_distance(a: &[&str], b: &[&str]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, a_word) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, b_word) in b.iter().enumerate() {
            let cost = if a_word.eq_ignore_ascii_case(b_word) { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

pub fn parse_verbose_json_has_timestamps(body: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let segment_timing = value
        .get("segments")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.iter().any(has_start_end))
        .unwrap_or(false);
    let word_timing = value
        .get("words")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.iter().any(has_start_end))
        .unwrap_or(false);
    segment_timing || word_timing
}

fn has_start_end(item: &serde_json::Value) -> bool {
    item.get("start").and_then(serde_json::Value::as_f64).is_some()
        && item.get("end").and_then(serde_json::Value::as_f64).is_some()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::parity`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/parity.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): WER + verbose_json timestamp helpers for parity check"
```

### Task 13: Dispatcher, results, session state, fallback

**Files:**
- Create: `desktop/src-tauri/src/stt/dispatch.rs`
- Modify: `desktop/src-tauri/src/stt/mod.rs`

The thin dispatcher (§5, §12): selects a backend, runs the batch, writes each sibling `<stem>.txt`, and returns per-file results. `PreferCrispasr` uses per-file crispasr while healthy and switches remaining files to a single Python batch on engine-down (so "unhealthy at first use" = the whole batch runs Python with one model-load). Forced modes surface engine-down as a top-level error. Also defines `SttState`, `SttCommandError`, and the Setup labels (§13).

- [ ] **Step 1: Write the failing test**

Add `pub mod dispatch;` to `desktop/src-tauri/src/stt/mod.rs`, then create `desktop/src-tauri/src/stt/dispatch.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct Scripted {
        queue: Mutex<VecDeque<Result<String, SttError>>>,
    }
    impl Scripted {
        fn new(items: Vec<Result<String, SttError>>) -> Self {
            Self { queue: Mutex::new(items.into_iter().collect()) }
        }
    }
    impl SttBackend for Scripted {
        fn transcribe(&self, _audio: &Path, _language: &str) -> Result<String, SttError> {
            self.queue.lock().unwrap().pop_front().unwrap_or(Err(SttError::AudioDecode))
        }
    }

    fn temp_paths(count: usize, tag: &str) -> Vec<String> {
        let base = std::env::temp_dir().join(format!("yap-dispatch-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        (0..count).map(|i| base.join(format!("clip{i}.wav")).display().to_string()).collect()
    }

    #[test]
    fn prefer_uses_crispasr_when_healthy_and_writes_sibling() {
        let paths = temp_paths(1, "healthy");
        let crispasr = Scripted::new(vec![Ok("crispasr text".into())]);
        let python = Scripted::new(vec![Ok("python text".into())]);
        let mut fell_back = false;
        let results =
            dispatch(BackendChoice::PreferCrispasr, &crispasr, &python, &mut fell_back, &paths, "en").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_none());
        assert!(!fell_back);
        let sibling = std::path::PathBuf::from(&paths[0]).with_extension("txt");
        assert_eq!(std::fs::read_to_string(sibling).unwrap().trim(), "crispasr text");
    }

    #[test]
    fn prefer_falls_back_to_python_when_engine_down() {
        let paths = temp_paths(2, "fallback");
        let crispasr = Scripted::new(vec![Err(SttError::SidecarUnreachable)]);
        let python = Scripted::new(vec![Ok("py one".into()), Ok("py two".into())]);
        let mut fell_back = false;
        let results =
            dispatch(BackendChoice::PreferCrispasr, &crispasr, &python, &mut fell_back, &paths, "en").unwrap();
        assert!(fell_back);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.error.is_none()));
    }

    #[test]
    fn per_file_audio_error_is_recorded_and_batch_continues() {
        let paths = temp_paths(2, "perfile");
        let crispasr = Scripted::new(vec![Err(SttError::AudioDecode), Ok("second".into())]);
        let python = Scripted::new(vec![]);
        let mut fell_back = false;
        let results =
            dispatch(BackendChoice::PreferCrispasr, &crispasr, &python, &mut fell_back, &paths, "en").unwrap();
        assert_eq!(results[0].error.as_deref(), Some("AUDIO_DECODE"));
        assert!(results[1].error.is_none());
        assert!(!fell_back);
    }

    #[test]
    fn forced_crispasr_surfaces_when_engine_down() {
        let paths = temp_paths(1, "forced");
        let crispasr = Scripted::new(vec![Err(SttError::SidecarUnreachable)]);
        let python = Scripted::new(vec![]);
        let mut fell_back = false;
        let err =
            dispatch(BackendChoice::Crispasr, &crispasr, &python, &mut fell_back, &paths, "en").unwrap_err();
        assert_eq!(err.code, "SIDECAR_UNREACHABLE");
    }

    #[test]
    fn engine_status_labels_map_states() {
        assert_eq!(engine_status_label(engine_readiness(true, false)), "Transcription engine ready");
        assert_eq!(engine_status_label(engine_readiness(true, true)), "Using Python fallback");
        assert_eq!(engine_status_label(engine_readiness(false, false)), "Transcription engine not installed yet");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test stt::dispatch`
Expected: FAIL — unresolved names (`dispatch`, `BackendChoice`, `engine_status_label`, …).

- [ ] **Step 3: Write the minimal implementation**

Add above the test module in `desktop/src-tauri/src/stt/dispatch.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::stt::backend::{select_backend, BackendChoice, SttBackend};
use crate::stt::crispasr::CrispasrBackend;
use crate::stt::error::SttError;
use crate::stt::python::PythonBackend;
use crate::stt::sidecar::CrispasrSidecar;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResult {
    pub input: String,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttCommandError {
    pub code: String,
    pub message: String,
}

impl From<SttError> for SttCommandError {
    fn from(error: SttError) -> Self {
        Self { code: error.code().to_string(), message: error.user_message().to_string() }
    }
}

pub struct SttState {
    pub sidecar: Arc<Mutex<CrispasrSidecar>>,
    fell_back: AtomicBool,
}

impl SttState {
    pub fn new() -> Self {
        Self { sidecar: Arc::new(Mutex::new(CrispasrSidecar::new())), fell_back: AtomicBool::new(false) }
    }

    pub fn set_fell_back(&self, value: bool) {
        self.fell_back.store(value, Ordering::Relaxed);
    }

    pub fn fell_back(&self) -> bool {
        self.fell_back.load(Ordering::Relaxed)
    }
}

impl Default for SttState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineReadiness {
    Ready,
    Fallback,
    NotInstalled,
}

pub fn engine_readiness(engine_ready: bool, fell_back: bool) -> EngineReadiness {
    if fell_back {
        EngineReadiness::Fallback
    } else if engine_ready {
        EngineReadiness::Ready
    } else {
        EngineReadiness::NotInstalled
    }
}

pub fn engine_status_label(state: EngineReadiness) -> &'static str {
    match state {
        EngineReadiness::Ready => "Transcription engine ready",
        EngineReadiness::Fallback => "Using Python fallback",
        EngineReadiness::NotInstalled => "Transcription engine not installed yet",
    }
}

pub fn engine_ready() -> bool {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let binary_ok = crate::stt::sidecar::resolve_binary(|key| std::env::var(key).ok(), &exe_dir).is_ok();
    let model_ok = crate::stt::pin::load_pin()
        .map(|pin| crate::stt::model::models_dir().join(pin.gguf_file).exists())
        .unwrap_or(false);
    binary_ok && model_ok
}

pub fn transcribe_paths(
    state: &SttState,
    root: PathBuf,
    paths: Vec<String>,
    language: &str,
) -> Result<Vec<TranscriptResult>, SttCommandError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let backend_env = std::env::var("YAP_STT_BACKEND").ok();
    let choice = select_backend(backend_env.as_deref());
    let crispasr = CrispasrBackend::new(Arc::clone(&state.sidecar));
    let python = PythonBackend::new(root);
    let mut fell_back = false;
    let result = dispatch(choice, &crispasr, &python, &mut fell_back, &paths, language);
    if fell_back {
        state.set_fell_back(true);
    }
    result
}

pub fn dispatch<C, P>(
    choice: BackendChoice,
    crispasr: &C,
    python: &P,
    fell_back: &mut bool,
    paths: &[String],
    language: &str,
) -> Result<Vec<TranscriptResult>, SttCommandError>
where
    C: SttBackend,
    P: SttBackend,
{
    let outcomes = match choice {
        BackendChoice::Python => run_forced(python, paths, language),
        BackendChoice::Crispasr => run_forced(crispasr, paths, language),
        BackendChoice::PreferCrispasr => Ok(run_prefer(crispasr, python, fell_back, paths, language)),
    };
    let outcomes = outcomes.map_err(SttCommandError::from)?;
    Ok(finalize(paths, outcomes))
}

fn run_forced<B: SttBackend>(
    backend: &B,
    paths: &[String],
    language: &str,
) -> Result<Vec<Result<String, SttError>>, SttError> {
    let files: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let outcomes = backend.transcribe_batch(&files, language);
    let all_engine_down =
        !outcomes.is_empty() && outcomes.iter().all(|outcome| matches!(outcome, Err(error) if is_engine_down(*error)));
    if all_engine_down {
        let error = outcomes
            .iter()
            .find_map(|outcome| outcome.as_ref().err().copied())
            .unwrap_or(SttError::SidecarUnreachable);
        Err(error)
    } else {
        Ok(outcomes)
    }
}

fn run_prefer<C: SttBackend, P: SttBackend>(
    crispasr: &C,
    python: &P,
    fell_back: &mut bool,
    paths: &[String],
    language: &str,
) -> Vec<Result<String, SttError>> {
    let mut outcomes: Vec<Option<Result<String, SttError>>> = vec![None; paths.len()];
    let mut switch_index: Option<usize> = None;
    for (index, path) in paths.iter().enumerate() {
        let audio = PathBuf::from(path);
        match crispasr.transcribe(&audio, language) {
            Ok(text) => outcomes[index] = Some(Ok(text)),
            Err(error) if is_engine_down(error) => {
                crate::stt::log_stt(&format!(
                    "crispasr unhealthy ({}); switching remaining files to python fallback",
                    error.code()
                ));
                *fell_back = true;
                switch_index = Some(index);
                break;
            }
            Err(error) => outcomes[index] = Some(Err(error)),
        }
    }
    if let Some(start) = switch_index {
        let remaining: Vec<PathBuf> = paths[start..].iter().map(PathBuf::from).collect();
        for (offset, outcome) in python.transcribe_batch(&remaining, language).into_iter().enumerate() {
            outcomes[start + offset] = Some(outcome);
        }
    }
    outcomes.into_iter().map(|outcome| outcome.unwrap_or(Err(SttError::AudioDecode))).collect()
}

fn is_engine_down(error: SttError) -> bool {
    match error {
        SttError::SidecarCrash | SttError::SidecarUnreachable => true,
        SttError::ModelMissing
        | SttError::ModelCorrupt
        | SttError::BadLang
        | SttError::Oom
        | SttError::AudioDecode
        | SttError::Busy
        | SttError::Timeout => false,
    }
}

fn finalize(paths: &[String], outcomes: Vec<Result<String, SttError>>) -> Vec<TranscriptResult> {
    paths
        .iter()
        .zip(outcomes)
        .map(|(path, outcome)| {
            let audio = PathBuf::from(path);
            match outcome.and_then(|text| write_sibling_txt(&audio, &text)) {
                Ok(output) => TranscriptResult { input: path.clone(), output: output.display().to_string(), error: None },
                Err(error) => TranscriptResult {
                    input: path.clone(),
                    output: String::new(),
                    error: Some(error.code().to_string()),
                },
            }
        })
        .collect()
}

fn write_sibling_txt(audio: &Path, text: &str) -> Result<PathBuf, SttError> {
    let output = audio.with_extension("txt");
    std::fs::write(&output, format!("{text}\n")).map_err(|_| SttError::AudioDecode)?;
    Ok(output)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::dispatch`
Expected: PASS (five dispatch tests).

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/dispatch.rs desktop/src-tauri/src/stt/mod.rs
git commit -m "feat(stt): dispatcher with prefer-crispasr auto-fallback + sibling writes"
```

### Task 14: Wire Tauri state + commands + kill-on-exit + idle monitor

**Files:**
- Modify: `desktop/src-tauri/src/lib.rs`

Rewire the two commands to delegate to `stt::dispatch`, register `SttState`, kill the child on app exit (`RunEvent::Exit`), and run the idle-unload monitor thread. Remove the now-dead Phase-0 helpers.

- [ ] **Step 1: Update the failing test**

In the `#[cfg(test)] mod tests` of `desktop/src-tauri/src/lib.rs`, replace `setup_status_serializes_for_frontend` with the version below, and **delete** the `command_failure_message_uses_traceback_tail` test:

```rust
    #[test]
    fn setup_status_serializes_for_frontend() {
        let value = serde_json::to_value(SetupStatus {
            model: "model".into(),
            root: "root".into(),
            python_ready: true,
            script_ready: true,
            python: "python.exe".into(),
            engine_ready: true,
            using_fallback: false,
            engine_status: "Transcription engine ready".into(),
        })
        .unwrap();

        assert_eq!(value["pythonReady"], true);
        assert_eq!(value["scriptReady"], true);
        assert_eq!(value["engineReady"], true);
        assert_eq!(value["usingFallback"], false);
        assert_eq!(value["engineStatus"], "Transcription engine ready");
        assert!(value.get("python_ready").is_none());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test --lib`
Expected: FAIL — `SetupStatus` has no field `engine_ready` (struct not updated yet).

- [ ] **Step 3: Update the `SetupStatus` struct**

Replace the `SetupStatus` struct in `desktop/src-tauri/src/lib.rs` with:

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupStatus {
    model: String,
    root: String,
    python_ready: bool,
    script_ready: bool,
    python: String,
    engine_ready: bool,
    using_fallback: bool,
    engine_status: String,
}
```

- [ ] **Step 4: Rewrite `setup_status` to report engine readiness**

Replace the `setup_status` command in `desktop/src-tauri/src/lib.rs` with:

```rust
#[tauri::command]
fn setup_status(state: tauri::State<'_, stt::dispatch::SttState>) -> SetupStatus {
    let root = project_root();
    let python = python_path(&root);
    let script = root.join("transcribe.py");
    let engine_ready = stt::dispatch::engine_ready();
    let using_fallback = state.fell_back();
    let readiness = stt::dispatch::engine_readiness(engine_ready, using_fallback);
    log_line(&format!(
        "setup_status engine_ready={engine_ready} using_fallback={using_fallback}"
    ));

    SetupStatus {
        model: std::env::var("YAP_MODEL_ID")
            .unwrap_or_else(|_| "ZoOtMcNoOt/yap-cohere-transcribe-03-2026".into()),
        root: root.display().to_string(),
        python_ready: python.exists(),
        script_ready: script.exists(),
        python: python.display().to_string(),
        engine_ready,
        using_fallback,
        engine_status: stt::dispatch::engine_status_label(readiness).to_string(),
    }
}
```

- [ ] **Step 5: Rewrite `transcribe_files` as a thin delegator**

Replace the entire `transcribe_files` command in `desktop/src-tauri/src/lib.rs` with:

```rust
#[tauri::command]
fn transcribe_files(
    state: tauri::State<'_, stt::dispatch::SttState>,
    paths: Vec<String>,
) -> Result<Vec<stt::dispatch::TranscriptResult>, stt::dispatch::SttCommandError> {
    log_line(&format!("transcribe_files count={}", paths.len()));
    stt::dispatch::transcribe_paths(&state, project_root(), paths, "en")
}
```

- [ ] **Step 6: Remove the dead Phase-0 helpers**

Delete these three items from `desktop/src-tauri/src/lib.rs` (all superseded by the `stt` module): the `TranscriptResult` struct, the `command_failure_message` function, and both `#[cfg(windows)]` / `#[cfg(not(windows))]` `hide_child_console` functions. Keep `project_root`, `python_path`, `log_line`, `read_text_file`, `write_polished_text`, `open_devtools`, and `polished_path`.

- [ ] **Step 7: Register state, kill-on-exit, and the idle monitor**

Replace the `run()` function in `desktop/src-tauri/src/lib.rs` with:

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|panic| {
        log_line(&format!("panic: {panic}"));
    }));
    log_line("app start");

    let stt_state = stt::dispatch::SttState::new();
    let sidecar_for_monitor = std::sync::Arc::clone(&stt_state.sidecar);
    let sidecar_for_exit = std::sync::Arc::clone(&stt_state.sidecar);

    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        if let Ok(mut sidecar) = sidecar_for_monitor.lock() {
            sidecar.unload_if_idle();
        }
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(stt_state)
        .invoke_handler(tauri::generate_handler![
            setup_status,
            transcribe_files,
            read_text_file,
            write_polished_text,
            open_devtools
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(move |_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                if let Ok(mut sidecar) = sidecar_for_exit.lock() {
                    sidecar.shutdown();
                }
            }
        });
}
```

- [ ] **Step 8: Run the full Rust test suite**

Run (in `desktop/src-tauri/`): `cargo test`
Expected: PASS — all `stt::*` module tests plus the remaining `lib.rs` tests (`setup_status_serializes_for_frontend`, `read_text_file_rejects_non_transcripts`, `polished_path_writes_sibling_file`). No warnings about unused `command_failure_message` / `hide_child_console`.

- [ ] **Step 9: Commit**

```bash
git add desktop/src-tauri/src/lib.rs
git commit -m "feat(stt): wire dispatcher into Tauri commands + kill sidecar on exit"
```

### Task 15: Frontend error→toast mapping + `transcribeFiles` wrapper

**Files:**
- Modify: `desktop/package.json`
- Create: `desktop/src/stt.ts`
- Create: `desktop/src/stt.test.ts`
- Modify: `desktop/src/App.tsx`

Mirror `polish.ts` (§13): an exhaustive code→message map (TS `switch` with a `never` default), a `transcribeFiles` wrapper that normalizes the `{code, message}` reject into an `SttInvokeError`, and per-file error surfacing. Adds `vitest` for the pure mapping test.

- [ ] **Step 1: Add the test runner**

Install vitest (run in `desktop/`): `npm install -D vitest`
Then add a `test` script to `desktop/package.json` `"scripts"`:

```json
"test": "vitest run"
```

- [ ] **Step 2: Write the failing test**

Create `desktop/src/stt.test.ts`:

```ts
import { describe, expect, it } from "vitest";

import { isSttErrorCode, SttInvokeError, sttErrorMessage } from "./stt";

describe("stt error mapping", () => {
  it("maps every known code to a non-empty message", () => {
    const codes = [
      "MODEL_MISSING", "MODEL_CORRUPT", "BAD_LANG", "OOM", "AUDIO_DECODE",
      "SIDECAR_CRASH", "SIDECAR_UNREACHABLE", "BUSY", "TIMEOUT",
    ] as const;
    for (const code of codes) {
      expect(sttErrorMessage(code).length).toBeGreaterThan(0);
    }
    expect(sttErrorMessage("SIDECAR_UNREACHABLE")).toBe("Transcription engine didn't start.");
  });

  it("recognizes known codes and rejects unknown ones", () => {
    expect(isSttErrorCode("BUSY")).toBe(true);
    expect(isSttErrorCode("NOPE")).toBe(false);
  });

  it("uses the mapped message for known codes and the detail otherwise", () => {
    expect(new SttInvokeError("MODEL_CORRUPT", "raw").message).toBe("Model file failed verification.");
    expect(new SttInvokeError("PYTHON_WEIRD", "raw detail").message).toBe("raw detail");
  });
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run (in `desktop/`): `npm test`
Expected: FAIL — cannot resolve `./stt`.

- [ ] **Step 4: Write the minimal implementation**

Create `desktop/src/stt.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";

export type SttErrorCode =
  | "MODEL_MISSING"
  | "MODEL_CORRUPT"
  | "BAD_LANG"
  | "OOM"
  | "AUDIO_DECODE"
  | "SIDECAR_CRASH"
  | "SIDECAR_UNREACHABLE"
  | "BUSY"
  | "TIMEOUT";

export type TranscriptResult = {
  input: string;
  output: string;
  error?: string;
};

export type SttFailure = {
  code: string;
  message: string;
};

const sttErrorCodes: readonly SttErrorCode[] = [
  "MODEL_MISSING",
  "MODEL_CORRUPT",
  "BAD_LANG",
  "OOM",
  "AUDIO_DECODE",
  "SIDECAR_CRASH",
  "SIDECAR_UNREACHABLE",
  "BUSY",
  "TIMEOUT",
];

export function isSttErrorCode(value: string): value is SttErrorCode {
  return (sttErrorCodes as readonly string[]).includes(value);
}

export function sttErrorMessage(code: SttErrorCode): string {
  switch (code) {
    case "MODEL_MISSING":
      return "Transcription model isn't installed yet.";
    case "MODEL_CORRUPT":
      return "Model file failed verification.";
    case "BAD_LANG":
      return "That language isn't supported.";
    case "OOM":
      return "Ran out of memory while transcribing.";
    case "AUDIO_DECODE":
      return "Couldn't read that audio file.";
    case "SIDECAR_CRASH":
      return "Transcription engine crashed.";
    case "SIDECAR_UNREACHABLE":
      return "Transcription engine didn't start.";
    case "BUSY":
      return "Transcription is busy — try again in a moment.";
    case "TIMEOUT":
      return "Transcription timed out.";
    default: {
      const exhaustive: never = code;
      return exhaustive;
    }
  }
}

export class SttInvokeError extends Error {
  code: string;
  detail: string;

  constructor(code: string, detail: string) {
    super(isSttErrorCode(code) ? sttErrorMessage(code) : detail || "Transcription failed.");
    this.name = "SttInvokeError";
    this.code = code;
    this.detail = detail;
  }
}

function toFailure(raw: unknown): SttFailure {
  if (raw && typeof raw === "object" && "code" in raw) {
    const failure = raw as { code?: unknown; message?: unknown };
    return {
      code: typeof failure.code === "string" ? failure.code : "",
      message: typeof failure.message === "string" ? failure.message : "",
    };
  }
  return { code: "", message: typeof raw === "string" ? raw : String(raw) };
}

export async function transcribeFiles(paths: string[]): Promise<TranscriptResult[]> {
  try {
    return await invoke<TranscriptResult[]>("transcribe_files", { paths });
  } catch (raw) {
    const failure = toFailure(raw);
    throw new SttInvokeError(failure.code, failure.message);
  }
}

export function transcriptFileError(result: TranscriptResult): string | undefined {
  if (!result.error) return undefined;
  return isSttErrorCode(result.error) ? sttErrorMessage(result.error) : result.error;
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run (in `desktop/`): `npm test`
Expected: PASS (three tests).

- [ ] **Step 6: Wire the wrapper into `App.tsx`**

In `desktop/src/App.tsx`, add this import near the top (with the other `@/` imports):

```ts
import { SttInvokeError, transcribeFiles, transcriptFileError } from "@/stt";
```

Extend the local `SetupStatus` type (lines ~45-51) to include the new fields:

```ts
type SetupStatus = {
  model: string;
  root: string;
  pythonReady: boolean;
  scriptReady: boolean;
  python: string;
  engineReady: boolean;
  usingFallback: boolean;
  engineStatus: string;
};
```

Delete the local `TranscriptResult` type (lines ~53-56); it is now imported from `@/stt` (and inferred from `transcribeFiles`).

Update the setup-status handler so the status reflects the engine (replace the `setStatus(...)` line after `invoke<SetupStatus>("setup_status")`):

```ts
      setStatus(
        setup.engineReady || (setup.pythonReady && setup.scriptReady)
          ? setup.engineStatus
          : "Setup missing",
      );
```

- [ ] **Step 7: Replace the transcription try/catch body**

In `transcribeItems`, replace everything from `const results = await invoke<TranscriptResult[]>("transcribe_files", {` through the `toast.error(message);` line inside `catch` with:

```ts
      const results = await transcribeFiles(pending.map((item) => item.path));
      const succeeded = results.filter((result) => !result.error);
      const failed = results.filter((result) => result.error);
      const outputs = new Map(succeeded.map((result) => [result.input, result.output]));
      const failedInputs = new Set(failed.map((result) => result.input));
      const texts: Record<string, string> = {};

      for (const result of succeeded) {
        try {
          texts[result.output] = await invoke<string>("read_text_file", { path: result.output });
        } catch {
          // ponytail: transcript can still be revealed if eager preview read fails.
        }
      }

      setQueue((items) =>
        items.map((item) => {
          if (outputs.has(item.path)) {
            return { ...item, output: outputs.get(item.path), status: "done" };
          }
          if (failedInputs.has(item.path)) {
            return { ...item, status: "error", error: "Transcription failed" };
          }
          return item;
        }),
      );
      recordHistoryEntries(
        pending.flatMap((item) => {
          const output = outputs.get(item.path);
          return output
            ? [
                {
                  createdAt: new Date().toISOString(),
                  name: item.name,
                  outputPath: output,
                  sourcePath: item.path,
                },
              ]
            : [];
        }),
      );
      setTranscriptText((current) => ({ ...current, ...texts }));
      for (const result of failed) {
        toast.error(transcriptFileError(result) ?? "Transcription failed.");
      }
      setStatus(failed.length ? "Needs attention" : "Ready");
      setAuth("Authorized");
      if (succeeded.length) {
        toast.success(`Transcribed ${succeeded.length} file${succeeded.length === 1 ? "" : "s"}`);
      }
    } catch (error) {
      const failure = error instanceof SttInvokeError ? error : undefined;
      const message = failure?.message ?? String(error || "Transcription failed");
      const detail = failure?.detail ?? message;
      setQueue((items) =>
        items.map((item) =>
          pending.some((pendingItem) => pendingItem.id === item.id)
            ? { ...item, status: "error", error: message }
            : item,
        ),
      );
      setStatus("Needs attention");
      setAuth(detail.includes("Hugging Face") ? "Run hf auth login" : "Check runner output");
      toast.error(message);
```

- [ ] **Step 8: Verify types + build**

Run (in `desktop/`): `npm run build`
Expected: PASS — `tsc` type-checks (no unused `TranscriptResult`, all fields resolve) and `vite build` succeeds.

- [ ] **Step 9: Commit**

```bash
git add desktop/package.json desktop/package-lock.json desktop/src/stt.ts desktop/src/stt.test.ts desktop/src/App.tsx
git commit -m "feat(stt): frontend error mapping + transcribeFiles wrapper with fallback status"
```

### Task 16: Parity spot-check + `verbose_json` probe (gated integration test)

**Files:**
- Create: `desktop/src-tauri/tests/parity.rs`

The trust-bar check (§12.1, §15): a WER-tolerant comparison of `crispasr` vs the Python path on a known clip, plus a one-call `verbose_json` probe confirming segment/word timing (for Phase 7). It is an integration test gated on `YAP_PARITY_CLIP` — it **skips** (and passes) when the env/binary/model aren't present, so the normal suite stays green; set the env to actually run it.

- [ ] **Step 1: Write the failing test**

Create `desktop/src-tauri/tests/parity.rs`:

```rust
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use desktop_lib::stt::backend::SttBackend;
use desktop_lib::stt::crispasr::CrispasrBackend;
use desktop_lib::stt::parity::{parse_verbose_json_has_timestamps, word_error_rate};
use desktop_lib::stt::python::PythonBackend;
use desktop_lib::stt::sidecar::CrispasrSidecar;

fn parity_clip() -> Option<PathBuf> {
    std::env::var("YAP_PARITY_CLIP").ok().map(PathBuf::from)
}

fn parity_root() -> PathBuf {
    std::env::var("YAP_PARITY_ROOT").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("../.."))
}

#[test]
fn crispasr_matches_python_within_wer_tolerance() {
    let Some(clip) = parity_clip() else {
        eprintln!("skipping parity: set YAP_PARITY_CLIP to a known audio clip");
        return;
    };

    let python = PythonBackend::new(parity_root());
    let python_text = python.transcribe(&clip, "en").expect("python transcription");

    let sidecar = Arc::new(Mutex::new(CrispasrSidecar::new()));
    let crispasr = CrispasrBackend::new(sidecar);
    let crispasr_text = crispasr.transcribe(&clip, "en").expect("crispasr transcription");

    let wer = word_error_rate(&python_text, &crispasr_text);
    println!("parity WER = {wer:.3}");
    assert!(wer <= 0.20, "WER {wer:.3} exceeds the 0.20 tolerance");
}

#[test]
fn crispasr_verbose_json_carries_timestamps() {
    let Some(clip) = parity_clip() else {
        eprintln!("skipping verbose_json probe: set YAP_PARITY_CLIP");
        return;
    };

    let sidecar = Arc::new(Mutex::new(CrispasrSidecar::new()));
    let base_url = sidecar.lock().unwrap().ensure_ready().expect("sidecar ready");

    let client = reqwest::blocking::Client::new();
    let form = reqwest::blocking::multipart::Form::new()
        .file("file", &clip)
        .expect("clip file")
        .text("language", "en")
        .text("response_format", "verbose_json");
    let body = client
        .post(format!("{base_url}/v1/audio/transcriptions"))
        .multipart(form)
        .send()
        .expect("verbose_json request")
        .text()
        .expect("verbose_json body");

    assert!(
        parse_verbose_json_has_timestamps(&body),
        "verbose_json response lacked segment/word timing: {body}"
    );
    sidecar.lock().unwrap().shutdown();
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test --test parity`
Expected: FAIL — `module stt is private` (the integration crate cannot see `desktop_lib::stt`).

- [ ] **Step 3: Expose the `stt` module to integration tests**

In `desktop/src-tauri/src/lib.rs`, change the module declaration from:

```rust
mod stt;
```

to:

```rust
pub mod stt;
```

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test --test parity`
Expected: PASS — both tests **skip** locally (no `YAP_PARITY_CLIP`) and the binary compiles against the public API.

To run the check for real (feeds the §12.1 trust bar), from `desktop/src-tauri/` with the venv, pinned binary, and model available:

```powershell
$env:YAP_CRISPASR_BIN = "C:\path\to\crispasr.exe"
$env:YAP_PARITY_CLIP = "C:\clips\known.wav"
cargo test --test parity -- --nocapture
```

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/lib.rs desktop/src-tauri/tests/parity.rs
git commit -m "test(stt): gated WER parity + verbose_json timestamp probe"
```

### Task 17: externalBin packaging + least-privilege capabilities + binary pin procedure

**Files:**
- Modify: `desktop/src-tauri/tauri.conf.json`
- Verify: `desktop/src-tauri/capabilities/default.json`
- Modify: `desktop/crispasr-version.txt`

Ship the pinned, hash-verified binary as a Tauri `externalBin` sidecar (§9) while keeping capabilities least-privilege (§10.3). We spawn via `std::process` (Task 10), so **no shell permission is added**.

- [ ] **Step 1: Declare the sidecar as an externalBin**

In `desktop/src-tauri/tauri.conf.json`, add an `externalBin` entry to the `bundle` object (place it directly above `"icon"`):

```json
    "externalBin": ["binaries/crispasr"],
```

The `bundle` object becomes:

```json
  "bundle": {
    "active": true,
    "targets": "all",
    "externalBin": ["binaries/crispasr"],
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "windows": {
      "nsis": {
        "installerHooks": "nsis-hooks.nsh"
      }
    }
  }
```

- [ ] **Step 2: Place the verified binary (packaging) / document the dev override**

Obtain the pinned `crispasr` binary per §9 (a GitHub release asset over HTTPS, or `build-windows.bat`), verify it, and place it where Tauri expects the target-triple name:

```powershell
# From desktop/src-tauri/
mkdir binaries -Force
Copy-Item C:\path\to\crispasr.exe .\binaries\crispasr-x86_64-pc-windows-msvc.exe
# Confirm the SHA-256 matches binary_sha256 in desktop/crispasr-version.txt:
(Get-FileHash -Algorithm SHA256 .\binaries\crispasr-x86_64-pc-windows-msvc.exe).Hash.ToLower()
```

Notes:
- **Runtime resolution (Task 8/10):** dev uses `YAP_CRISPASR_BIN`; the bundled app resolves `crispasr.exe` next to the main executable (Tauri strips the target-triple suffix when bundling). Both are SHA-256 re-verified before spawn.
- `tauri build` (packaging) requires `binaries/crispasr-<target-triple>.exe` to exist. Plain `cargo build`, `npm run build`, and CI (Task 18) do **not** bundle, so they are unaffected if the binary is absent during the dev slice.

- [ ] **Step 3: Verify capabilities stay minimal**

Confirm `desktop/src-tauri/capabilities/default.json` is **unchanged** — no `shell:*` or process permissions were added (we do not use `tauri-plugin-shell`). It must remain:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "core:window:allow-close",
    "core:window:allow-minimize",
    "core:window:allow-start-dragging",
    "core:window:allow-toggle-maximize",
    "dialog:default",
    "opener:default"
  ]
}
```

- [ ] **Step 4: Verify config + build**

Run (in `desktop/src-tauri/`): `cargo build`
Expected: PASS. Also confirm `tauri.conf.json` is valid JSON (no trailing-comma errors).

> **Release fast-follow (NOT in this plan, §10.4):** code-sign + notarize the app **and** the nested `crispasr.exe` (Windows Authenticode via Azure Trusted Signing) and OS-sandbox the sidecar before shipping to users. Until then, the pinned SHA-256 is the primary integrity gate.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/tauri.conf.json
git commit -m "build(stt): declare crispasr externalBin sidecar (least-privilege spawn)"
```

### Task 18: CI supply-chain hygiene

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `.github/dependabot.yml`

Counter the Shai-Hulud install-script worm class and dependency drift (§10.3): honor lockfiles (`npm ci`, `cargo build --locked`), run `npm audit` / `cargo audit`, and enable Dependabot. Lockfiles (`Cargo.lock`, `package-lock.json`) already exist and are refreshed by Tasks 1 + 15.

- [ ] **Step 1: Create the CI workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  frontend:
    runs-on: windows-latest
    defaults:
      run:
        working-directory: desktop
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm
          cache-dependency-path: desktop/package-lock.json
      - run: npm ci
      - run: npm audit --audit-level=high
      - run: npm test
      - run: npm run build

  rust:
    runs-on: windows-latest
    defaults:
      run:
        working-directory: desktop/src-tauri
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --locked
      - run: cargo test --locked
      - name: cargo audit
        run: |
          cargo install cargo-audit --locked
          cargo audit
```

> Supply-chain note (§10.3): `npm ci --ignore-scripts` is preferred, but the current toolchain (esbuild/vite) needs its install script, so we rely on the committed lockfile + `npm audit` here. Revisit `--ignore-scripts` if the toolchain allows.

- [ ] **Step 2: Enable Dependabot**

Create `.github/dependabot.yml`:

```yaml
version: 2
updates:
  - package-ecosystem: npm
    directory: /desktop
    schedule:
      interval: weekly
  - package-ecosystem: cargo
    directory: /desktop/src-tauri
    schedule:
      interval: weekly
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
```

- [ ] **Step 3: Verify the pinned commands locally**

Run (in `desktop/`): `npm ci` — Expected: PASS (installs exactly from `package-lock.json`).
Run (in `desktop/src-tauri/`): `cargo build --locked` — Expected: PASS (no `Cargo.lock` changes needed).
Run (in `desktop/src-tauri/`): `cargo install cargo-audit --locked; cargo audit` — Expected: completes (address any advisories it reports).
Run (in `desktop/`): `npm audit --audit-level=high` — Expected: completes (address high/critical advisories).

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml .github/dependabot.yml
git commit -m "ci(stt): lockfile-pinned builds + npm/cargo audit + dependabot"
```

### Task 19: Corrupt cached GGUF repair (delete + re-download)

**Files:**
- Modify: `desktop/src-tauri/src/stt/model.rs`

**Origin:** Task 5 leaves a gap — when a cached GGUF exists but `verify_sha256` fails (`ModelCorrupt`), the file stays on disk and every subsequent run fails without re-downloading. This task closes that loop: delete the corrupt cache entry and fall through to the download path (same fail-closed policy as a bad fresh download).

- [ ] **Step 1: Write the failing test**

Add inside the existing `tests` module in `desktop/src-tauri/src/stt/model.rs`:

```rust
    #[test]
    fn ensure_model_deletes_corrupt_cache_and_redownloads() {
        let dir = std::env::temp_dir().join(format!("yap-dl-corrupt-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.gguf"), b"tampered-on-disk").unwrap();
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let pin = sample_pin(hello);
        let mut download_calls = 0;
        let dest = ensure_model_at(&dir, &pin, |_url, path| {
            download_calls += 1;
            std::fs::write(path, b"hello").map_err(|_| SttError::ModelMissing)
        })
        .unwrap();
        assert_eq!(download_calls, 1, "must re-download after deleting corrupt cache");
        assert!(dest.exists());
        assert!(verify_sha256(&dest, hello).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run (in `desktop/src-tauri/`): `cargo test ensure_model_deletes_corrupt_cache_and_redownloads`
Expected: FAIL — corrupt cache returns `ModelCorrupt` without re-downloading (or test panics because download never runs).

- [ ] **Step 3: Fix `ensure_model_at` cache branch**

In `ensure_model_at`, replace the early-return on cache hit:

```rust
    if dest.exists() {
        verify_sha256(&dest, &pin.gguf_sha256)?;
        return Ok(dest);
    }
```

with:

```rust
    if dest.exists() {
        match verify_sha256(&dest, &pin.gguf_sha256) {
            Ok(()) => return Ok(dest),
            Err(SttError::ModelCorrupt) => {
                let _ = std::fs::remove_file(&dest);
                // ponytail: fall through to download path — same repair as post-download mismatch
            }
            Err(err) => return Err(err),
        }
    }
```

Keep the post-download mismatch delete unchanged (no duplication beyond the shared `verify_sha256` call).

- [ ] **Step 4: Run the test to verify it passes**

Run (in `desktop/src-tauri/`): `cargo test stt::model`
Expected: PASS (9 tests).

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/stt/model.rs
git commit -m "fix(stt): delete corrupt cached GGUF and re-download"
```

---

## Self-Review

Performed after all tasks were written; issues found were fixed inline.

### 1. Spec coverage

| Spec section | Task(s) |
| --- | --- |
| §2 Decision (Approach A: sidecar behind trait, Python fallback) | 6, 7, 10, 11 |
| §3 Reconciled API (`/health` cohere-gate, `/v1/audio/transcriptions`) | 8, 9, 11 |
| §4 Bucket 2 (in scope) | all build tasks; Buckets 1 & 3 in **Out of Scope** |
| §5 Architecture (thin dispatcher, trait, both backends, Rust writes `<stem>.txt`) | 6, 7, 11, 13 |
| §6.1 Health ready-gate | 8 (`health_is_ready`), 10 (`wait_ready`) |
| §6.2 Batch happy path + tolerant parse | 9 (`parse_transcription_json`), 11 (`post_transcription`) |
| §6.3 `/load`, §6.4 `/inference` | **Not built** — spec keeps them as future/documented alternatives (accommodated by the trait) |
| §7 Lifecycle (lazy spawn, port 8765→8775, 10s ready-gate, one-in-flight, restart-once+retry-once, idle-unload 10min, kill on exit, log) | 8, 10, 11, 14 |
| §8 Model cache + pinned download + SHA-256 verify (fail-closed); offline run; **corrupt cache repair** | 3, 4, 5, **19** (+ §8 "offline": launch with local `-m <gguf>`, no auto-download flags — see note below) |
| §9 Binary acquisition (externalBin, version pin, `YAP_CRISPASR_BIN`, verify before spawn) | 3, 8, 10, 17 |
| §10.3 Controls (pin+hash binary & GGUF, scrubbed env, loopback+ephemeral port, validate inputs, CI hygiene) | 3, 4, 5, 8, 9, 10, 17, 18 |
| §10.4 Signing + OS sandbox | **Out of Scope** (release fast-follow, noted once) |
| §10.5 Security acceptance (fail-closed, loopback, no-secret env, python fallback works) | 4, 5, 8, 10, 7, 13 |
| §11 Error contract (`SttError` exhaustive + toast map) | 2, 13, 15 |
| §12 Migration (`YAP_STT_BACKEND`, prefer+auto-fallback, rollback via forced python) | 6, 13 |
| §12.1 Trust bar (parity + crash-recovery) | 16 (parity/verbose_json), 10 + 11 (restart/retry) |
| §13 Settings/UI (engine-ready, fallback status, error toasts) | 14, 15 |
| §14 Acceptance criteria | 10, 11, 13, 14, 16 (functional); 4, 5, 8, 10 (security) |
| §15 Testing (backend dispatch unit test; WER parity; CI smoke deferred) | 6 + 13 (dispatch), 16 (parity); CI smoke **deferred** per spec |
| §17 Resolved decisions (endpoint `/v1`, Rust-owned fetch, prebuilt externalBin, integrity anchor `crispasr-version.txt`) | 3, 5, 9, 11, 17 |

No in-scope spec requirement is left without a task. Deliberate non-builds (§6.3/§6.4, §10.4, Buckets 1 & 3) are recorded in **Out of Scope** / **Design Notes**, matching the spec.

**§8 offline note:** the sidecar runs offline because it is launched with a **local, pre-verified** `-m <gguf>` and **no** `--hf-repo` / `-m auto` flags (`build_launch_args`, Task 8), so it never fetches at runtime. Hard egress-blocking (a firewall/sandbox) is the §10.4 fast-follow.

### 2. Placeholder scan

Searched for `TBD` / `TODO` / `FIXME` / `similar to Task` / `handle edge cases` / `write tests for the above` / `... more code`: **none found**. Every code step contains complete, real code.

The only fill-at-build values are in `desktop/crispasr-version.txt` (`binary_sha256`, `gguf_revision`, `gguf_sha256`). Per spec §17 these are **genuine values, not placeholders**: Task 3 gives the exact PowerShell commands to produce them, and `parse_pin` (Task 3) + `verify_sha256` (Task 4) reject anything that isn't a real 64-hex hash — the code fails closed, so a bogus value cannot ship.

### 3. Type consistency

Verified names/signatures agree across all tasks:

- `SttError` — 9 variants + `code()` / `user_message()` used identically in Rust (Tasks 2, 7, 9, 10, 11, 13) and mirrored in TS `SttErrorCode` / `sttErrorMessage` (Task 15).
- `SttBackend::transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError>` + default `transcribe_batch`; overridden only by `PythonBackend` (Task 7). No accidental recursion (`PythonBackend::transcribe` → its own `transcribe_batch` override; `CrispasrBackend` uses the default loop).
- `BackendChoice` / `select_backend` (Task 6) consumed in `dispatch` (Task 13).
- `CrispasrPin` field names identical in `pin.rs` (Task 3), `model.rs` (Task 5), `sidecar.rs` (Task 10), `dispatch.rs` (Task 13).
- `CrispasrSidecar` methods (`new`, `ensure_ready`, `restart`, `mark_used`, `unload_if_idle`, `shutdown`, `base_url`, `is_running`) called consistently by `CrispasrBackend` (Task 11) and `lib.rs` (Task 14).
- `CrispasrBackend::new(Arc<Mutex<CrispasrSidecar>>)` used identically in Tasks 11, 13, 16.
- `TranscriptResult { input, output, error? }` and `SttCommandError { code, message }` (Task 13) match the `lib.rs` command return type (Task 14) and the TS types (Task 15).
- Two distinct, intentionally-named error predicates: `is_sidecar_failure` (retryable, incl. `Timeout`; `crispasr.rs`) vs `is_engine_down` (fallback trigger, excl. `Timeout`; `dispatch.rs`).
- All Rust `match` over enums are exhaustive (no variant-hiding `_`); the TS `sttErrorMessage` switch has a `never` default.

**Fix applied during review:** ensured the module-root helpers (`hide_child_console`, `log_stt`) live once in `stt/mod.rs` (Task 7) and are referenced as `crate::stt::…` by `python.rs`, `sidecar.rs`, and `dispatch.rs`; the duplicate `hide_child_console` in `lib.rs` is removed in Task 14 to avoid a dead-code warning.
