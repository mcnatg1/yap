# Yap Desktop

Tauri desktop app for the local Moonshine fallback sidecar. See `..\README.md` for the repo map.

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

Playwright writes traces, videos, screenshots, and visual diffs under `test-results` on failure.
The WebdriverIO/Tauri smoke test uses the debug binary at `src-tauri\target\debug\desktop.exe`
unless `APP_BINARY` is set. Run WDIO under Node 24 LTS; Node 26 has produced embedded-session
failures on this machine.
