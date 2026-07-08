# Yap Desktop

Tauri desktop app for the local Nemotron INT8 fallback. Yap targets installed desktop
windows, not phone/mobile layouts. See `..\README.md` for the repo map.

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
