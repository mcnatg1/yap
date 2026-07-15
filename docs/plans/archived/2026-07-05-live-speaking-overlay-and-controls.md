# Live Speaking Overlay And Controls Implementation Plan

> **Historical record — current authority (2026-07-14):** This foundation plan
> predates the converged island and safety amendments. Any synthesized
> `SendInput` delivery described below is retired; completed transcripts now use
> clipboard-only delivery after native-confirmed bounded shortcut enrollment.
> Use [ADR 0013](../../adr/0013-global-hotkey-injection.md) and the
> [live client spec](../../specs/live-dictation-client-ux.md) as authority.

> **Implementation status (2026-07-14):** Historical foundation plan. Overlay, hotkeys, capture, local streaming, recording, native chord enrollment, and Windows clipboard delivery subsequently landed; the original exclusions below describe that PR's scope, not current product capability.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the next PR foundation for a snappy always-available live speaking overlay, configurable recording hotkey, microphone setup, and typed live state hooks without implementing cross-app text injection, full live STT streaming, Scribe, save-session audio, or the Phase 8 server connector.

**Historical design:** [Live speaking overlay and controls](../../archive/historical-designs/2026-07-05-live-speaking-overlay-and-controls.md)

**Architecture:** Tauri Rust owns live session state and OS hooks. React renders a compact overlay and settings projection from typed snapshots. The overlay is a separate window/webview labeled `live-overlay`; capture is hotkey-gated or explicitly started; route state exposes `serverLive` as a future typed route, `localFallback`, `none`, or blocked. Without the Phase 8 connector, the app must not claim active server streaming.

**Tech Stack:** Tauri 2, Rust, React 19, TypeScript, GSAP dynamic imports for transform/opacity overlay polish, `tauri-plugin-global-shortcut`, `cpal`, existing Vitest/Cargo tests.

---

## File Structure

- Modify `docs/specs/live-dictation-client-ux.md`: scope amendment for overlay/hotkey foundation.
- Modify `docs/adr/0013-global-hotkey-injection.md`: clarify that injection remains later while overlay/hotkey settings can land earlier.
- Modify `desktop/src/lib/app-types.ts`: live session, route, hotkey, mic device projection types and label helpers.
- Create `desktop/src/lib/live-session.test.ts`: pure state/label tests.
- Create `desktop/src/components/live/live-overlay.tsx`: compact and expanded overlay UI.
- Create `desktop/src/components/live/live-overlay-host.tsx`: host shell for the overlay window.
- Modify `desktop/src/App.tsx`: main-window state projection and settings wiring only.
- Modify `desktop/src/components/panels/app-sheets.tsx`: live controls settings section.
- Create `desktop/src-tauri/src/live/mod.rs`, `devices.rs`, `hotkeys.rs`, `settings.rs`, `state.rs`: Rust live state, persisted preferences, and OS hooks.
- Create `desktop/src/settings.ts`: frontend invoke wrappers if live/settings calls start to crowd `App.tsx`.
- Modify `desktop/src-tauri/src/lib.rs`: manage live state, register commands, create/show overlay window.
- Modify `desktop/src-tauri/Cargo.toml`: add live-hook dependencies only when the Rust slice uses them.
- Modify `desktop/src-tauri/capabilities/default.json`: add only the minimum window/global-shortcut permissions required by the chosen Tauri APIs.

## Constraints

- Do not implement cross-app text injection.
- Do not add always-listening behavior.
- Do not route live capture through the recordings queue except when saving a completed live session.
- Do not add server WSS streaming in this PR; expose route state and connector seams only.
- Do not add full audio-thread/STT streaming, Scribe, or saved live audio unless this plan is explicitly split into a larger implementation branch.
- Do not add `@gsap/react` unless the current dynamic import pattern becomes insufficient.
- Keep UI copy terse; settings may show compact operational details, but docs carry the architecture.
- Respect `prefers-reduced-motion` in overlay motion.

---

### Task 1: Reconcile Existing Docs

**Files:**
- Modify `docs/specs/live-dictation-client-ux.md`
- Modify `docs/adr/0013-global-hotkey-injection.md`

- [ ] Add a scope amendment to Phase 3: overlay/hotkey foundation is in scope; cross-app injection remains out of scope.
- [ ] Link to the new Superpowers spec.
- [ ] Clarify ADR 0013 sequencing: hotkey registration and overlay settings may ship before injection.
- [ ] Run:

```powershell
cd C:\dev\cohere-transcribe-local
rg -n "global hotkey|injection|overlay|live-overlay" docs/specs/live-dictation-client-ux.md docs/adr/0013-global-hotkey-injection.md docs/archive/historical-designs/2026-07-05-live-speaking-overlay-and-controls.md
```

Expected: docs consistently distinguish overlay/hotkey foundation from injection.

---

### Task 2: Type The Live Session Projection

**Files:**
- Modify `desktop/src/lib/app-types.ts`
- Create `desktop/src/lib/live-session.test.ts`

- [ ] Add `LiveOverlayVisibility`, `LiveCaptureMode`, `LiveSessionStatus`, `LiveRoute`, `LiveSessionView`, and `LiveInputDeviceView`.
- [ ] Include `visibility` on `LiveSessionView` and support `route: "none"` for hidden/idle states.
- [ ] Add pure label helpers for route/status.
- [ ] Add tests for route labels, blocked state labels, and reduced-motion-independent state data.
- [ ] Run:

```powershell
cd C:\dev\cohere-transcribe-local\desktop
pnpm test -- src/lib/live-session.test.ts
```

Expected: focused test passes.

---

### Task 3: Build Overlay UI In React First

**Files:**
- Create `desktop/src/components/live/live-overlay.tsx`
- Create `desktop/src/components/live/live-overlay-host.tsx`
- Modify `desktop/src/main.tsx` or routing bootstrap only if needed for a second window entry.

- [ ] Render collapsed dot/pill, expanded controls, and blocked/error states from `LiveSessionView`.
- [ ] Use GSAP dynamic import only for transform/opacity morph between collapsed and expanded tiers.
- [ ] Use `window.matchMedia("(prefers-reduced-motion: reduce)")` to skip GSAP motion.
- [ ] Keep text minimal: route, partial/final snippet, stop/save buttons, blocked action.
- [ ] Add keyboard and screen-reader labels for mic state, stop, save, and settings.
- [ ] Run:

```powershell
cd C:\dev\cohere-transcribe-local\desktop
pnpm test
pnpm build
```

Expected: UI compiles and existing tests pass.

---

### Task 4: Add Tauri Overlay Window Commands

**Files:**
- Create/modify `desktop/src-tauri/src/live/state.rs`
- Modify `desktop/src-tauri/src/lib.rs`
- Modify `desktop/src-tauri/tauri.conf.json`
- Modify `desktop/src-tauri/capabilities/default.json`

- [ ] Add Rust-owned `LiveSessionState` with status, route, hotkey, device, level, text, and error fields.
- [ ] Add commands/events: `live_status`, `show_live_overlay`, `hide_live_overlay`, `set_live_overlay_enabled`.
- [ ] Create or retrieve a `live-overlay` webview/window with transparent, undecorated, always-on-top top-center positioning.
- [ ] Position against the active monitor, use explicit top/safe-margin constants, and avoid title-bar/system control overlap where possible.
- [ ] Keep overlay window non-invasive: no taskbar entry where supported and no focus stealing on passive state.
- [ ] Run:

```powershell
cd C:\dev\cohere-transcribe-local
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml live
cargo clippy --locked --manifest-path desktop\src-tauri\Cargo.toml --all-targets -- -D warnings
```

Expected: live state tests pass and clippy is clean.

---

### Task 5: Add Configurable Hotkey Foundation

**Files:**
- Modify `desktop/src-tauri/Cargo.toml`
- Create `desktop/src-tauri/src/live/hotkeys.rs`
- Create/modify `desktop/src-tauri/src/live/settings.rs`
- Modify `desktop/src-tauri/src/lib.rs`
- Modify `desktop/src/components/panels/app-sheets.tsx`

- [ ] Add `tauri-plugin-global-shortcut` only here, when registration is implemented.
- [ ] Default shortcut is `Ctrl+Shift+Space` on Windows until user testing changes it.
- [ ] Store selected chord and capture mode in app settings.
- [ ] Commands: `get_live_hotkey`, `set_live_hotkey`, `set_live_capture_mode`, `clear_live_hotkey`.
- [ ] Validate invalid chords separately from OS registration conflicts.
- [ ] On registration failure, preserve previous shortcut and return a typed error.
- [ ] Settings UI supports record shortcut, clear, reset, push-to-talk, and toggle mode.
- [ ] Run:

```powershell
cd C:\dev\cohere-transcribe-local
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml hotkey
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml live_settings
cd C:\dev\cohere-transcribe-local\desktop
pnpm build
```

Expected: hotkey validation tests pass and settings compiles.

---

### Task 6: Add Mic Device Foundation

**Files:**
- Modify `desktop/src-tauri/Cargo.toml`
- Create `desktop/src-tauri/src/live/devices.rs`
- Create/modify `desktop/src-tauri/src/live/settings.rs`
- Modify `desktop/src-tauri/src/live/state.rs`
- Modify `desktop/src/components/panels/app-sheets.tsx`

- [ ] Add `cpal` only here, when device enumeration is implemented.
- [ ] Commands: `list_input_devices`, `set_input_device`, `preflight_input_device`.
- [ ] Return default device, selected device, missing-device fallback, and preflight level result.
- [ ] Persist the selected input device and recover to the system default when it disappears.
- [ ] First live start requests microphone access through the chosen capture implementation and maps denied access to a typed `MIC_DENIED`/blocked state.
- [ ] Settings UI shows a compact device picker and "No input detected" state.
- [ ] Run:

```powershell
cd C:\dev\cohere-transcribe-local
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml devices
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml mic_permission
cargo clippy --locked --manifest-path desktop\src-tauri\Cargo.toml --all-targets -- -D warnings
```

Expected: device selection logic is tested without requiring a physical mic in CI.

---

### Task 7: Wire Start/Stop Live Intents Without Full STT Streaming

**Files:**
- Modify `desktop/src-tauri/src/live/state.rs`
- Modify `desktop/src-tauri/src/lib.rs`
- Modify `desktop/src/components/live/live-overlay.tsx`
- Modify `desktop/src/App.tsx`

- [ ] Commands: `start_live_session`, `stop_live_session`, `save_live_session`.
- [ ] Starting live runs route checks: Phase 8 server connector present and ready -> `serverLive`; fallback ready -> `localFallback`; neither -> blocked. If the server connector is not implemented, never claim active `serverLive` streaming.
- [ ] Add a tested transition for route loss: `serverLive` -> `localFallback` when fallback is ready, otherwise `blocked`, with a snapshot emitted to overlay and settings.
- [ ] The command emits state snapshots to main and overlay windows.
- [ ] Stop returns to idle and makes mic-hot state false.
- [ ] Save persists a real live WAV/TXT only when an actual audio buffer or transcript exists.
- [ ] Run:

```powershell
cd C:\dev\cohere-transcribe-local
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml live
cd C:\dev\cohere-transcribe-local\desktop
pnpm test
```

Expected: live control state works without pretending server streaming is complete.

---

### Task 8: Verify Product Surface

- [ ] Run full checks:

```powershell
cd C:\dev\cohere-transcribe-local\desktop
pnpm build
pnpm test
cd C:\dev\cohere-transcribe-local
cargo clippy --locked --manifest-path desktop\src-tauri\Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path desktop\src-tauri\Cargo.toml
git diff --check
```

- [ ] Manually verify:
  - Narrow window can open navigation and settings.
  - Overlay shows idle, listening, speaking, blocked, and saving states.
  - Reduced motion disables pulses/morphs.
  - Hotkey conflict errors are visible.
  - Mic missing/denied states lead back to settings.
  - Route labels distinguish server from fallback.
  - Mid-session route loss visibly downgrades from server to fallback or blocked.
  - Visible overlay does not open the mic while idle.
  - Hotkey capture stores only the chosen chord/mode.

## Spec Coverage Review

| Spec area | Covered by plan |
|-----------|-----------------|
| Always-available top overlay | Tasks 3-4 |
| Compact dot tier and expanded controls | Task 3 |
| Snappy GSAP motion with reduced-motion escape | Task 3 |
| Selectable keybinds | Task 5 |
| Mic permissions/device selection/autoselect | Task 6 |
| Tauri OS hooks and overlay window | Tasks 4-6 |
| Live state machine hook points | Tasks 2, 4, 7 |
| Server/live fallback route visibility | Tasks 2, 7 |
| Privacy and accessibility | Tasks 3, 5, 6, 8 |
| No injection in this PR | Constraints and Tasks 1, 7 |
| No Phase 8 server streaming claim | Constraints and Task 7 |

## Self-Review

- The plan changes existing docs and code paths instead of adding a detached readiness layer.
- The plan keeps Phase 3 live UX and ADR 0013 aligned by explicitly splitting hotkey/overlay foundation from text injection.
- The plan does not require server WSS, diarization, Scribe, saved live audio, or full audio streaming before the overlay foundation can be reviewed.
- The plan delays new Rust dependencies until the tasks that actually use them.
