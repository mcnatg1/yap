# Yap Desktop

Tauri desktop app for the local Nemotron INT8 fallback. Yap targets installed desktop
windows, not phone/mobile layouts. See `..\README.md` for the repo map.

`desktop/src-tauri/src/audio/` owns the source-aware capture foundation: callback-safe framing and loss accounting, deterministic preprocessing, explicit timelines/gaps, independently bounded sink fan-out, crash-safe recording/recovery, and evidence/result contracts. Local Nemotron decoding remains under `live/` and `stt/`; server ASR, diarization inference, and other model-heavy processing remain deferred.

Repo-owned Windows automation and installer validation require PowerShell Core 7.4 or newer (`pwsh.exe`). The scripts fail fast under legacy Windows PowerShell or an older Core runtime.

```powershell
cd C:\dev\cohere-transcribe-local\desktop
node -v  # should be v24.x
pnpm install
pnpm test
pnpm build
pnpm test:e2e
pnpm tauri dev
```

Desktop-level automation:

```powershell
pnpm test:e2e:update
pnpm test:desktop:build
pnpm test:desktop
```

Playwright and WDIO write traces, videos, screenshots, and visual diffs under `tests\results` on failure.
The WebdriverIO/Tauri smoke test uses the debug binary at `src-tauri\target\debug\yap-desktop.exe`
unless `APP_BINARY` is set. Run WDIO under Node 24 LTS; Node 26 has produced embedded-session
failures on this machine.

## Windows installer validation

Use the test identity for routine validation on a workstation. It installs as `Yap.Test`, uses
`com.mcnatg1.yap.test`, and routes Yap-owned runtime data to `%LOCALAPPDATA%\Yap.Test`.
The script verifies install, launch, notices, process cleanup, default-uninstall preservation, and
installer footprint cleanup. It refuses a production `Yap` installer before executing it.

```powershell
pnpm build:nsis:test
pnpm test:nsis:local
```

The local-safe path never invokes `/DELETEAPPDATA`. Test-owned recursive cleanup requires an exact
child path, a valid `.yap-test-tree-sentinel`, no reparse points, and a non-production leaf name.
Do not manually repoint it at `%LOCALAPPDATA%\Yap` or `com.mcnatg1.yap`.

The same `Yap.Test` artifact can exercise destructive uninstall semantics without touching the
production namespace:

```powershell
pnpm test:nsis:test-delete
```

Production-data deletion is a separate destructive verification mode. CI runs it on a fresh
GitHub-hosted Windows VM. It may also run inside Windows Sandbox, or from a dedicated disposable
Windows account named `YapTest*` or `YapSmoke*` whose profile contains this marker:

```powershell
Set-Content "$env:USERPROFILE\.yap-disposable-test-profile" "yap-disposable-profile-v1" -Encoding ascii
pnpm tauri build --bundles nsis
pwsh.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File .\tests\scripts\smoke-nsis-production-delete.ps1
```

Never create that disposable-profile marker in an everyday account. `RUNNER_ENVIRONMENT` is only
an accidental-safety guardrail; artifact identity, clean-footprint checks, exact target validation,
the per-run uninstall sentinel, and disposable-profile isolation are the real layers.
