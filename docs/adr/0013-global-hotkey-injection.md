# ADR 0013: Global hotkey + cross-app text injection (L1)

**Date:** 2026-06-30
**Status:** Accepted (roadmap — overlay/hotkey foundation in Phase 3; injection in Phase 7+)
**Builds on:** [ADR 0002](0002-crispasr-unified-stt-runtime.md) (STT sidecar), [ADR 0006](0006-silero-agents-state-machine.md) (orchestrator/pre-warm), [live spec](../specs/live-dictation-client-ux.md)

## Context

L1 of the Voice OS is a **global hotkey** that opens the mic from any app and **injects** the transcribed text into the focused field — the Wispr-Flow-style surface. ADR 0003 left open: "Does global hotkey/injector share the Yap Tauri process or a second tray app?"

The implementation now splits this surface into two deliverable layers:

- **Phase 3 foundation:** live overlay, configurable capture hotkey, mic settings, and live session state. This supports in-app/overlay live recording but does not inject text into other apps.
- **Phase 7+ injection:** OS accessibility/input permissions, focus detection, text insertion, and clipboard fallback.

Text injection remains the **last and most ambitious** surface and must not be promised on v1 ([ADR 0002](0002-crispasr-unified-stt-runtime.md), `PRODUCT.md` anti-references).

## Decision

### Process model — **same Tauri process + tray**, not a separate app

| Concern | Decision |
|---------|----------|
| Host | Existing Yap Tauri process, running in **tray/background** mode; no second binary |
| Hotkey | `tauri-plugin-global-shortcut` registers a user-configurable chord; capture controls can ship before text injection |
| Mic/STT | Reuses the warm **crispasr moonshine** path + orchestrator pre-warm ([ADR 0006](0006-silero-agents-state-machine.md)) — no second STT stack |
| Overlay | Small always-on-top webview (ghost preview), currently specified as a top-positioned translucent overlay in the Phase 3 foundation |
| Injection | Phase 7+ OS-level text insertion (e.g. `enigo`/platform APIs): paste-style insert into the focused field |

Reusing one process keeps **one STT sidecar, one orchestrator, one model residency** — the whole point of ADR 0006. A separate tray app would duplicate sidecar management and risk dual model loads.

### Permissions (explicit, per-OS)
- **macOS:** Accessibility (for injection) + Input Monitoring (global hotkey) + Microphone — request with clear rationale; degrade to "copy to clipboard" if denied.
- **Windows:** global hotkey + mic; injection via SendInput; UIPI limits on elevated targets → clipboard fallback.
- Never silently capture: overlay clearly shows when the mic is hot.

### Fallback ladder
```
inject into focused field → if blocked → copy to clipboard + toast "Copied — paste with Ctrl/Cmd+V"
```

### Scope guard
- English-only (live policy); multilingual hotkey waits on the multilingual-live ADR.
- No keylogging / no always-listening; hotkey-gated capture only.

## Consequences

### Positive
- Single process → single sidecar/orchestrator; consistent with ADR 0006 invariants.
- Reuses live pipeline (Phase 3); L1 is mostly hotkey + overlay + injection glue.
- Clear permission + fallback story protects the local-first trust model.

### Negative
- OS injection + accessibility permissions are the **highest-friction, most fragile** surface (per-OS quirks, elevated windows).
- Tray/background mode adds lifecycle states (running while window closed).

### Neutral
- Phase 7+; explicitly **not** a v1 promise. Compete on batch + local + live-in-app first ([ADR 0002](0002-crispasr-unified-stt-runtime.md)).

## Alternatives considered
- **Separate tray/daemon app** — rejected: duplicates sidecar mgmt; risks dual STT residency; two binaries to sign/update.
- **Clipboard-only (no injection)** — viable **fallback**, rejected as the primary because it isn't the Wispr-class UX the surface promises.
- **Accessibility-tree typing per app** — rejected v1: brittle across apps; OS insert + clipboard fallback is more robust.

## References
- [ADR 0006](0006-silero-agents-state-machine.md) — pre-warm, single residency
- [live spec](../specs/live-dictation-client-ux.md) — the pipeline L1 reuses
- [ADR 0003](0003-long-term-voice-architecture.md) — original open question resolved here
