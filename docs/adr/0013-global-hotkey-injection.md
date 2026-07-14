# ADR 0013: Global hotkey + safe cross-app delivery (L1)

**Date:** 2026-06-30
**Status:** Accepted as amended 2026-07-14 (single process, native hotkeys,
tray-owned overlay, and clipboard delivery active on Windows; synthesized
focused-field input is retired until exact field authority can be proven)
**Builds on:** [ADR 0006](0006-silero-agents-state-machine.md)
(orchestrator/pre-warm), [ADR 0019](0019-local-streaming-model-selection.md)
(in-process Nemotron), [live spec](../specs/live-dictation-client-ux.md)

## Context

L1 of the Voice OS is a global hotkey that opens the mic from any app and makes
the completed transcript immediately available to the user. ADR 0003 left open
whether that surface should share the Yap Tauri process or use a second tray
app.

The implementation keeps one exact-bounds tray-owned live island, safe shipped
dictation/paste-last defaults, microphone settings, live session state, and a
dedicated last-completed transcript. The server may improve transcript quality,
but it never owns OS input, focus, or delivery.

The original ADR allowed stop-time foreground-window capture followed by
Unicode `SendInput`. Security review established that a foreground HWND or
process identity does not prove authority over the exact focused text field
after asynchronous decoding. That behavior is superseded. Yap now copies the
completed transcript to the clipboard and shows manual-paste guidance; it does
not synthesize keyboard input.

## Decision

### Process model — same Tauri process + tray

| Concern | Decision |
|---|---|
| Host | Existing Yap Tauri process in tray/background mode; no second binary. |
| Hotkeys | `tauri-plugin-global-shortcut` registers separate dictation (`Ctrl+Shift+Space`) and paste-last (`Ctrl+Shift+Alt+V`) defaults. Changing a shortcut requires a native confirmation dialog and one bounded 15-second Windows physical-chord epoch. Capture waits for a neutral keyboard, accepts one complete press-and-release chord, ignores ordinary typing that lacks the required modifiers, and persists only the normalized chord. Dictation requires at least two modifiers; paste-last requires three. Rust rejects unsupported, reserved, or conflicting chords and replaces registrations transactionally while idle. |
| Mic/STT | Reuse the warm in-process Nemotron `LiveStreamEngine` selected by ADR 0019; there is one client-local recognizer runtime and no second ASR stack. |
| Overlay | Reuse one always-on-top, non-focusable top-bezel WebView. React requests semantic collapsed/expanded/status surfaces; Rust alone owns exact native bounds, position, and the rounded Windows interaction region. |
| Delivery | Copy a cleaned completed transcript to the Windows clipboard using a valid Yap HWND owner, then show visible manual-paste status. Paste-last recopies only the dedicated last-completed transcript. Yap does not call `SendInput` or claim authority over an external focused field. |

Reusing one process keeps one client-local recognizer, one orchestrator, and one
model residency, consistent with ADR 0006. A separate tray app would duplicate
recognizer management and risk dual model loads.

### Permissions and disclosure

- Windows uses the global-hotkey API, microphone access, and clipboard access;
  it does not require accessibility-tree access or synthesized input.
- macOS/Linux global-hotkey, microphone, and clipboard adapters remain
  follow-on work and must request only the permissions their implementation
  actually needs.
- The overlay clearly shows when the microphone is hot and when the transcript
  has been copied for manual paste.

### Delivery ladder

```text
final transcript -> normalize -> clipboard with Yap owner -> visible manual-paste status
```

Direct insertion may be reconsidered only if a future adapter can capture and
revalidate authority over the exact destination field and passes security plus
real-application compatibility tests. A foreground window or process ID alone
is insufficient.

### Scope guard

- English-only live policy; multilingual hotkey behavior waits on a dedicated
  multilingual-live decision.
- No keylogging or always-listening. Physical shortcut enrollment exists only
  after native user confirmation, lasts at most 15 seconds, requires a
  neutral/chord/release sequence, and cannot run concurrently with another
  enrollment.
- Raw keyboard events are neither logged nor persisted. Only the validated,
  normalized final chord is stored.
- Shortcut settings expose explicit Change, Cancel, and per-action Reset states.

## Consequences

### Positive

- Single process preserves one recognizer/orchestrator lifecycle.
- Native bounded enrollment avoids renderer-wide keyboard capture and prevents
  ordinary typing from becoming a shortcut.
- Clipboard-only delivery has an honest authority boundary and keeps completed
  text recoverable without injecting into the wrong field.

### Negative

- The user performs the final paste gesture; this is less automatic than the
  original Wispr-style target.
- Windows ships first. macOS/Linux adapters and a broad real-application matrix
  remain follow-on work.

### Neutral

- A safer direct-insertion adapter is deferred, not prohibited, but it must
  prove exact-field authority before this decision can be amended again.

## Alternatives considered

- **Separate tray/daemon app** — rejected: duplicates recognizer management,
  risks dual STT residency, and adds a second binary to sign and update.
- **Foreground-window capture plus `SendInput`** — superseded: foreground
  process identity cannot revalidate the exact destination field after decode.
- **Accessibility-tree typing per app** — deferred: potentially stronger field
  identity, but permission-heavy and brittle across applications; it requires
  separate design, threat review, and compatibility evidence.

## References

- [ADR 0006](0006-silero-agents-state-machine.md) — pre-warm and single residency
- [live spec](../specs/live-dictation-client-ux.md) — the pipeline L1 reuses
- [ADR 0003](0003-long-term-voice-architecture.md) — original process question
