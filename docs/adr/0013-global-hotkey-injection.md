# ADR 0013: Global hotkey + cross-app text injection (L1)

**Date:** 2026-06-30
**Status:** Accepted (Windows adapter active; native target smoke and cross-platform adapters continue)
**Builds on:** [ADR 0006](0006-silero-agents-state-machine.md) (orchestrator/pre-warm), [ADR 0019](0019-local-streaming-model-selection.md) (in-process Nemotron), [live spec](../specs/live-dictation-client-ux.md)

## Context

L1 of the Voice OS is a **global hotkey** that opens the mic from any app and **injects** the transcribed text into the stop-time focused external control — the Wispr-Flow-style surface. ADR 0003 left open: "Does global hotkey/injector share the Yap Tauri process or a second tray app?"

The implementation now splits this surface into two deliverable layers:

- **Current desktop path:** one exact-bounds tray-owned live island, explicitly armed physical-code shortcut settings with shipped dictation/paste-last defaults, mic settings, live session state, automatic Windows insertion after completion, and paste-last recovery.
- **Cross-platform hardening:** macOS accessibility integration, richer permission recovery, application compatibility probes, and explicit fallback feedback.

Focused-field injection is a core dictation behavior. It must remain client-owned, local, and independent of server availability. The server may improve transcript quality, but it must not own OS focus or insertion.

## Decision

### Process model — **same Tauri process + tray**, not a separate app

| Concern | Decision |
|---------|----------|
| Host | Existing Yap Tauri process, running in **tray/background** mode; no second binary |
| Hotkey | `tauri-plugin-global-shortcut` registers separate dictation (`Ctrl+Shift+Space`) and paste-last (`Ctrl+Shift+Alt+V`) defaults. Settings record `KeyboardEvent.code` only after explicit arming; Rust normalizes the final chord, rejects unsupported/reserved/conflicting values, and replaces registrations transactionally while idle. |
| Mic/STT | Reuses the warm in-process Nemotron `LiveStreamEngine` selected by ADR 0019 — one client-local recognizer runtime, no second ASR stack |
| Overlay | One continuously reused, always-on-top, non-focusable top-bezel webview. React requests semantic collapsed/expanded/status surfaces; Rust alone owns exact native bounds, position, and the rounded Windows interaction region. |
| Injection | Capture the external foreground window and, when Windows exposes it, the focused child control when stop begins; revalidate the available target data after final decoding, then send Unicode input. If focus changed, modifiers remain held, or UIPI blocks input, write the full transcript to the clipboard with a valid Yap HWND owner and surface manual-paste status. |

Reusing one process keeps **one client-local recognizer, one orchestrator, and one model residency** — the whole point of ADR 0006. A separate tray app would duplicate recognizer management and risk dual model loads.

### Permissions (explicit, per-OS)
- **macOS:** Accessibility (for injection) + Input Monitoring (global hotkey) + Microphone — request with clear rationale; degrade to "copy to clipboard" if denied.
- **Windows:** global hotkey + mic; injection via `SendInput`; UIPI limits on elevated targets leave the transcript on the clipboard for manual paste.
- Never silently capture: overlay clearly shows when the mic is hot.

### Fallback ladder
```
capture stop-time target → revalidate after decode → Unicode `SendInput` → if unsafe/blocked, copy full text + show manual-paste status
```

### Scope guard
- English-only (live policy); multilingual hotkey waits on the multilingual-live ADR.
- No keylogging / no always-listening; hotkey-gated capture only.
- Shortcut recording has explicit Change, Cancel, and per-action Reset states. It observes no keyboard events outside the armed recorder and never logs or persists raw events.

## Consequences

### Positive
- Single process → single local recognizer/orchestrator; consistent with ADR 0006 and ADR 0019 invariants.
- Reuses the current client live pipeline; L1 is mostly hotkey + overlay + injection glue.
- Clear permission + fallback story protects the local-first trust model.

### Negative
- OS injection + accessibility permissions are the **highest-friction, most fragile** surface (per-OS quirks, elevated windows).
- Tray/background mode adds lifecycle states (running while window closed).

### Neutral
- Windows ships first. macOS/Linux adapters and deeper compatibility probes remain follow-on work.

## Alternatives considered
- **Separate tray/daemon app** — rejected: duplicates recognizer management; risks dual STT residency; two binaries to sign/update.
- **Clipboard-only (no injection)** — viable **fallback**, rejected as the primary because it isn't the Wispr-class UX the surface promises.
- **Accessibility-tree typing per app** — rejected v1: brittle across apps; OS insert + clipboard fallback is more robust.

## References
- [ADR 0006](0006-silero-agents-state-machine.md) — pre-warm, single residency
- [live spec](../specs/live-dictation-client-ux.md) — the pipeline L1 reuses
- [ADR 0003](0003-long-term-voice-architecture.md) — original open question resolved here
