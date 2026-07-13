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

Yap uses Tauri's stock NSIS template and canonical app-data path. On Windows, runtime data lives at
`%APPDATA%\com.mcnatg1.yap`; the stock uninstaller owns its normal UI and delete-data behavior.
There are no Yap-specific NSIS hooks, delete tokens, quarantine directories, or test installer
identity.

Builds and static release-contract checks may run on a normal workstation. The install/launch/
uninstall lifecycle may not: it mutates the real production installer identity and is therefore
bounded by a fresh GitHub-hosted Windows runner, Windows Sandbox, or another disposable Windows VM.

```powershell
pnpm tauri build --bundles nsis
$env:YAP_DISPOSABLE_WINDOWS = "1" # only inside the disposable VM
pnpm test:nsis:disposable
```

The harness requires a clean profile, verifies the exact installer hash when supplied, launches the
installed app until it creates `%APPDATA%\com.mcnatg1.yap\logs\yap.log`, bounds and reaps every
process it starts, runs stock silent uninstall, and confirms that stock silent uninstall preserves
app data. It never recursively deletes application data; disposal of the Windows environment is the
lifecycle cleanup boundary. Never set `YAP_DISPOSABLE_WINDOWS=1` in an everyday account.
