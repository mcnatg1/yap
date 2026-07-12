# Freeflow and Meetily donor audit

**Status:** Accepted research baseline; no donor code incorporated by this audit

**As of:** 2026-07-12 at Yap `51931e5c79fe5efbeb03c83c5b3cb1d7b33ccb7e`

**Scope:** Compare the user-supplied Freeflow and Meetily repositories with the
current Yap client/server boundary, then define what may be reused and what must
be rejected.

## Decision

Yap remains the implementation foundation. Its Rust/Tauri capture, bounded
sinks, exact gap accounting, crash-safe artifacts, Windows injection,
authorization, installer, and release evidence are stronger than either donor.

The supplied repositories are selective donors:

- **Freeflow** is the behavior reference for a low-friction dictation HUD,
  physical shortcut capture, app-context injection recovery, and a small
  protocol-driven pipeline. It is not a window implementation donor.
- **Meetily** is the workflow reference for meetings, device selection, imported
  audio, progress/cancellation, transcript review, and a SQLite-backed catalog.
  It is not an audio-runtime, security-policy, tray-ownership, or packaging
  donor.
- No wholesale copy, broad transpilation, backend rewrite, branding import, or
  model/binary import is authorized by this audit. Each selected slice first
  needs a behavior contract, a license closure, and a Yap-owned test.

This is a controlled convergence, not a restart: preserve Yap's safety and
performance wins, recreate the best donor behavior in Yap's current stack, and
directly adapt source only when that is cheaper and safer than reimplementation.

## Pinned source and verification truth

| Source | Exact revision | Root license | License SHA-256 | Stack/platform | Verification on this Windows host |
|--------|----------------|--------------|----------------|----------------|-----------------------------------|
| [mrinalwadhwa/freeflow](https://github.com/mrinalwadhwa/freeflow/tree/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0) | `7f96ccd37ee7f525a4bdf05aaed70deec13d97d0` | Apache-2.0 | `df0b155f45ac44a82f9f0fabd4f95a003e4727e981d320f4701f4d8b47a0fe85` | Native macOS 14+; Swift 5.9, AppKit/SwiftUI, AVFoundation/CoreAudio, WebKit | Source and tests inspected; checkout clean; build/tests not executable because `swift`, `xcodebuild`, `xcodegen`, and `make` are absent and AppKit requires macOS |
| [Zackriya-Solutions/meetily](https://github.com/Zackriya-Solutions/meetily/tree/0281737d87d26352fb0adc78c8c0975f691b23d1) | `0281737d87d26352fb0adc78c8c0975f691b23d1` | MIT | `a6b53a54af406b28309d724b10014e1b65b5fb6e863e1c7ec71448759bf62381` | Tauri 2/Rust, Next.js 14/React 18, SQLite; Windows/macOS/Linux targets | `pnpm install --frozen-lockfile` and production frontend build passed; one standalone Node test passed; lint is interactive/not CI-ready; native Cargo build is blocked by missing libclang/toolchain requirements |

The Freeflow repository supplied for this audit is **not Electron and not MIT**.
It is also not the already-attributed FreeFlow source in Yap. Yap currently
ships adaptations attributed to `zachlatta/freeflow` at revision
`7427ca982c19746770f5357ced16e993f2eb27fd` under MIT. Future provenance must
use distinct identities such as `freeflow-mrinalwadhwa` and
`freeflow-zachlatta`; their licenses and source histories must never be merged.

Donor-reported test counts and hosted workflow files are not treated as passing
evidence. Only commands run against these pinned checkouts are reported above.

## Current Yap baseline

| Domain | Yap authority to preserve | Current gap donors may help shape |
|--------|---------------------------|-----------------------------------|
| Island/window/tray | React projects typed live state; Tauri owns one non-focusable, always-on-top, taskbar-free window, Windows no-activate flags, native interaction regions, monitor following, recovery, and close-to-tray | The native frame stays `260x40`; the idle interaction region is a `260x8` sensor while the visible capsule is narrower. Collapsed/expanded bounds do not yet match visible content. |
| Shortcuts/injection | Rust parses gesture semantics, dispatches global shortcuts, transactionally registers/reverts settings, captures and revalidates the Windows target, inserts Unicode text, and falls back visibly through the clipboard | Settings accept typed strings; paste-last has no default; there is no deliberate physical-chord recorder, explicit Cancel, or per-action Reset. macOS/Linux injection adapters remain absent. |
| Audio/devices | Preallocated callback buffers, non-blocking bounded sinks, exact loss intervals, source-aware tracks, 16 kHz mono preprocessing, crash-safe recording, and immutable evidence/result contracts | No production Windows system loopback, durable hot-plug identity, Silero path, Opus transport, or meeting capture UX. |
| Persistence/recovery | Rust owns canonical artifacts, commits, partial recovery, deletion intent, quarantine, and authorized file access | Imported jobs and their numeric IDs remain a React/localStorage projection. A Rust SQLite job ledger must precede upload/drain. |
| Runtime/server | Typed route/orchestrator vocabulary blocks large jobs instead of pretending local success; the server has tested health/router value objects | No OpenAPI contract, network health endpoint, capability-aware connector, SQLite job ledger, HTTP/WSS runtime, upload/resume, worker, TLS, or auth. |
| Tests/release | Vitest, Playwright, native Tauri WDIO, Rust/Python tests, dependency audits, NSIS smoke, immutable release evidence, and file-level third-party provenance | No licensed real-speech/WER or RTTM fixture, Windows loopback hardware lane, cross-platform native matrix, or server connector/auth end-to-end test. |

The relevant Yap owners are `desktop/src-tauri/src/live/`,
`desktop/src-tauri/src/audio/`, `desktop/src-tauri/src/runtime/`,
`desktop/src/components/live/`, `desktop/src/components/panels/`, and `server/`.
React remains a projection; Rust remains runtime and durable-data authority.

## Primary donor source slices

Future implementation work should start from these pinned files rather than
searching either repository broadly:

| Donor | Slice | Pinned source | Use |
|-------|-------|---------------|-----|
| Freeflow | HUD native behavior | [`HUDOverlayWindow.swift`](https://github.com/mrinalwadhwa/freeflow/blob/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0/FreeFlowApp/Sources/HUDOverlayWindow.swift) | Negative geometry reference; click-through and nonactivation cases |
| Freeflow | HUD ownership/state | [`HUDController.swift`](https://github.com/mrinalwadhwa/freeflow/blob/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0/FreeFlowApp/Sources/HUDController.swift), [`HUDViewModel.swift`](https://github.com/mrinalwadhwa/freeflow/blob/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0/FreeFlowApp/Sources/HUDViewModel.swift) | Visual-state derivation, hover/processing task cancellation, paste-last recovery |
| Freeflow | Physical shortcut capture | [`settings.html`](https://github.com/mrinalwadhwa/freeflow/blob/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0/FreeFlowApp/Resources/settings.html) | Narrow algorithm donor; fix Cancel/current-label and add explicit Reset |
| Freeflow | Hotkey semantics | [`CGEventTapHotkeyProvider.swift`](https://github.com/mrinalwadhwa/freeflow/blob/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0/FreeFlowKit/Sources/FreeFlowKit/Services/CGEventTapHotkeyProvider.swift), [`ShortcutBinding.swift`](https://github.com/mrinalwadhwa/freeflow/blob/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0/FreeFlowKit/Sources/FreeFlowKit/Models/ShortcutBinding.swift) | Behavior/test cases only; macOS runtime code is rejected |
| Freeflow | Dictation recovery | [`DictationPipeline.swift`](https://github.com/mrinalwadhwa/freeflow/blob/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0/FreeFlowKit/Sources/FreeFlowKit/Services/DictationPipeline.swift), [`AppTextInjector.swift`](https://github.com/mrinalwadhwa/freeflow/blob/7f96ccd37ee7f525a4bdf05aaed70deec13d97d0/FreeFlowKit/Sources/FreeFlowKit/Services/AppTextInjector.swift) | Timeout/fallback and compatibility-matrix reference |
| Meetily | Tray workflow | [`tray.rs`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src-tauri/src/tray.rs) | State labels only; reject frontend evaluation/session storage authority |
| Meetily | Device workflow | [`DeviceSelection.tsx`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src/components/DeviceSelection.tsx), [`device_monitor.rs`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src-tauri/src/audio/device_monitor.rs) | Picker, refresh, level, disconnect/reconnect cases |
| Meetily | Import workflow | [`ImportAudioDialog.tsx`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src/components/ImportAudio/ImportAudioDialog.tsx), [`import.rs`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src-tauri/src/audio/import.rs) | Staged progress/cancel behavior; replace authority and persistence |
| Meetily | Recovery workflow | [`TranscriptRecovery.tsx`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src/components/TranscriptRecovery/TranscriptRecovery.tsx), [`incremental_saver.rs`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src-tauri/src/audio/incremental_saver.rs) | UX reference only; retain Yap's Rust journal/commit implementation |
| Meetily | Catalog shape | [`database/`](https://github.com/Zackriya-Solutions/meetily/tree/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src-tauri/src/database) | Schema/repository comparison for the Yap SQLite ledger |
| Meetily | Security boundary | [`tauri.conf.json`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src-tauri/tauri.conf.json), [`lib.rs`](https://github.com/Zackriya-Solutions/meetily/blob/0281737d87d26352fb0adc78c8c0975f691b23d1/frontend/src-tauri/src/lib.rs) | Negative capability/command-surface reference |

## Reuse map

The reuse class is deliberately conservative:

- **Concept:** reproduce behavior from a clean Yap contract.
- **Adapt:** port a narrow algorithm or component after provenance and parity tests.
- **Direct:** retain donor source substantially; file-level attribution is mandatory.
- **Reject:** do not import.

| Domain | Freeflow evidence at pinned revision | Meetily evidence at pinned revision | Yap decision and landing owner | Reuse class / cost |
|--------|--------------------------------------|------------------------------------|--------------------------------|--------------------|
| App composition and tray | `AppDelegate` creates one service graph, tray owner, pipeline, HUD, settings, and permissions | Tauri tray exposes start/pause/resume/stop, but also focuses/evaluates the webview and uses frontend `sessionStorage` to start work | Keep Yap's `app.rs`/`tray.rs` ownership. Borrow only the explicit state labels and single-instance UX. Never let the tray and React become competing recording authorities. | Concept / low |
| Island state and feedback | `HUDViewModel` derives visual state and uses cancellable hover/processing timers; `HUDContentView` supplies minimized, recording, processing, error, and recovery feedback | No comparable system-integrated dictation island | Port the state vocabulary and feedback timing into Yap's existing typed overlay projection. Keep reduced-motion support. | Concept, small algorithm adaptation / medium |
| Native island geometry | `HUDOverlayWindow` is a fixed `400x240` transparent bottom-centered panel; it polls the pointer at about 60 Hz and toggles click-through using hand-built hit rectangles | Conventional decorated `1100x700` main window | Reject both geometries. Implement a top-centered Yap window whose native bounds exactly equal the visible collapsed or expanded island and whose top edge remains anchored while it opens downward. | Reject donor code / high |
| Hover behavior | Enter/exit use cancellable delays (`600 ms` enter, `200 ms` exit); global polling compensates for the oversized transparent window | None | Adapt the cancellable grace pattern, not the values or global-poll-first design. Target hover-to-expanded p95 at or below `220 ms`; retain a measured exit grace so the target does not disappear mid-movement. | Concept / medium |
| Shortcut recorder | `settings.html` records physical `KeyboardEvent.code`, keeps modifier side, handles dead keys, previews chords, rejects bare printable keys, and commits a normalized chord | No global shortcut implementation | Adapt the recorder algorithm into React/TypeScript and Yap IPC. Do not paste it unchanged: Escape/outside-click resets the label to the static HTML default rather than restoring a custom current label, and it has no per-shortcut Reset flow. | Adapt / medium |
| Shortcut defaults and registration | Right Option hold-to-talk plus hands-free/paste/private defaults; runtime reads persisted bindings | None | Preserve Yap's transactional unregister/register/rollback. Ship documented dictation and paste-last defaults, reject reserved/conflicting/bare printable chords, preserve the old working binding on failure, and expose explicit Change, Cancel, and Reset actions. | Concept / medium |
| Text injection and recovery | App-specific Accessibility writes, clipboard-preserving paste, Unicode fallback, fresh context on paste-last, and transcript restoration after a failed retry | No cross-app dictation injection | Use strategy ordering as a compatibility-test inventory. Preserve Yap's stop-time foreground/focus revalidation and visible fallback. Do not port macOS Accessibility code to Windows. | Concept / medium |
| Device selection and preflight | CoreAudio device selection, device-change rebuild, live level stream, and disconnect fallback | Mic/system selectors, refresh, level concepts, backend selector, playback-device warnings, and reconnect workflow | Port the user workflow and state labels. Keep native device identity and lifecycle in Rust; replace name/index identity with a durable platform identifier where available. Do not log raw device events or expose an unbounded backend selector. | UI concept / medium |
| Mic and system capture | Strong macOS mic path; not a Windows system-audio donor | Multiple audio generations. One public system-capture path explicitly bails outside macOS while another treats output devices as CPAL input streams. Windows behavior was not proven on hardware. | Keep Yap's bounded capture core. Use Meetily only as a list of cases to test. Implement Windows loopback against a pinned, understood API and require real dual-source hardware proof before enabling meeting mode. | Research only / high |
| Audio preprocessing | Reusable-engine mic capture, mono/16 kHz conversion, level/far-field heuristics, silence gates | Resampling, mixing, normalization, VAD, reconnect, and checkpoint concepts, but production code contains unsafe `Send`, unbounded channels, contradictory constants/comments, and an unused placeholder `audio_v2` | Preserve Yap's exact-gap, bounded-sink pipeline. Port no donor audio subsystem wholesale. Adapt only independently benchmarked pure transforms whose behavior tests beat the current implementation. | Algorithm-by-algorithm / high |
| Imported audio | No meeting import workflow | File selection/validation, staged progress, cancellation, decode/resample/VAD/transcribe/save sequence, meeting-folder output | Recreate the staged UX and typed progress contract. Rust must admit/canonicalize the selected path, mint the job ID, persist the ledger row, and atomically publish outputs. Do not let import silently run the client fallback for ordinary large recordings. | Concept and narrow UI adaptation / medium |
| Recovery and durable catalog | Transcript buffer is memory-only | SQLite catalog plus human-readable meeting folders, IndexedDB recovery UI, and 30-second MP4 checkpoints | Adopt the inspectable catalog/folder product idea, not the implementation. Yap's Rust commit/journal/recovery remains authority; Phase 3 adds SQLite jobs. Reject frontend IndexedDB as canonical recovery state and reject non-hashed FFmpeg checkpoint concatenation. | Concept / medium |
| Meeting review and retranscription | Dictation-oriented only | Meeting list/detail, audio player, transcript preview, virtualized transcript, summary panes, retranscription/import dialogs | Selectively adapt layout and workflow after accessibility/performance review. Keep Yap's authorized media registry and immutable raw transcript/revision model. Avoid the coupled editor stack; Meetily's meeting-details route is `563 kB` and `785 kB` first-load JS in the audited build. | UI concept / medium-high |
| Local models and polish | Direct OpenAI BYOK plus Apple-Silicon Parakeet/MLX/Qwen paths | Whisper/Parakeet/Ollama/llama-helper plus model download managers | Reject as runtime donors. Yap keeps Nemotron fallback and the planned private server boundary. Model URLs, converted artifacts, adapters, and helper binaries require separate license, hash, and threat review. | Reject / high |
| Server boundary | No application backend daemon; cloud calls are direct | Legacy Python backend is explicitly archived; current app is local Tauri plus optional model/API processes | Neither defines Yap's server interface. Implement the existing canonical Phase 3 OpenAPI, private capability health, connector, and durable job ledger from Yap ADRs/specs. | None |
| Packaging/updating/assets | Sparkle/Xcode/signing and macOS symbols | Broad Tauri capabilities, unsigned/unverified downloader paths, updater, bundled FFmpeg workflow, icons/logos, and a tracked Visual Studio installer bootstrap | Preserve Yap's NSIS and immutable release evidence. Reject donor installers, downloaders, update keys, icons, logos, screenshots, fonts, model binaries, rendered platform symbols, and branding. | Reject |

## Explicit rejection findings

These findings prevent “copy everything and tighten it later” from being a safe
sequence:

### Freeflow

- The fixed transparent `400x240` panel is the same class of invisible-window
  compromise Yap is replacing. Its pass-through logic reduces click theft but
  does not make native bounds truthful.
- The `600 ms` hover delay misses the intended system-like response target.
- Global pointer polling and a global click monitor are compensations for the
  oversized native frame, not the desired architecture.
- AppKit, CGEventTap, Accessibility, CoreAudio, Keychain, Sparkle, and Apple
  model code are platform-specific and do not belong in Yap's Windows/Tauri
  client.
- Direct OpenAI BYOK, Apple-only Parakeet/MLX, remote in-app messages, the
  Freeflow name/icon, and model artifacts do not match Yap's product boundary.

### Meetily

- Its Tauri capabilities grant broad filesystem read/write access. Yap keeps
  command-level path authorization and narrow capabilities.
- Recording state is duplicated across Rust flags, the audio manager, React,
  tray intermediate state, events, and `sessionStorage`/webview evaluation.
- `audio_v2`, `core-old`, `recording_saver_old`, `lib_old_complex`, and backup
  files show unresolved generations; `audio_v2` is not wired and still contains
  placeholder methods.
- The audio path uses unsafe `Send` declarations and unbounded channels, while
  Yap already has bounded sinks and exact gap accounting.
- Windows system audio is contradictory and was not proven: one path says it is
  unimplemented outside macOS, while another relies on output-device-as-input
  behavior.
- The build/download path fetches external FFmpeg/tool artifacts without the
  immutable provenance standard Yap requires. The tracked
  `frontend/vs_buildtools.exe` is never reusable.
- The root MIT license does not close dependencies, models, codecs, fonts,
  icons, borrowed upstream code, or downloaded binaries. There is no complete
  third-party notice mapping.
- The current frontend build passes, but lint is interactive, two tests depend
  on undeclared Bun, and the native suite did not compile on this host.

## Convergence acceptance contracts

### One tray-owned island

- Exactly one continuously reused `live-overlay` native window exists.
- Collapsed and expanded native bounds equal the visible surface; no transparent
  area outside that surface receives pointer input.
- Hover expands the same window downward without activating or focusing Yap.
- Moving away starts a short cancellable collapse grace. Re-entry cancels it.
- The foreground application and focused control are unchanged by hover,
  expansion, button use, collapse, and ordinary recording transitions.
- Hover-to-expanded latency is measured; target p95 is at most `220 ms` on the
  reference Windows hardware.
- Reduced-motion mode removes nonessential movement without changing state or
  timing ownership.

Required evidence: pure state tests, Playwright visual/motion tests, native WDIO
geometry/focus/pass-through tests, multi-monitor/DPI tests, and repeated
expand/collapse stability with CPU/RSS sampling.

### Safe shortcuts

- Dictation and paste-last have documented defaults immediately after install.
- Recording starts only after an explicit **Change shortcut** action.
- The recorder observes physical key identity only while armed, ignores repeat,
  never logs or persists raw key events, and stores only the normalized chord.
- **Cancel** restores the current binding; **Reset** is a separate explicit
  action that restores the shipped default.
- Bare printable, OS-reserved, duplicate, and failed registrations are rejected.
- Replacement is transactional: a failure leaves the prior working binding
  registered and persisted.

Required evidence: pure normalization/conflict tests, registration rollback
tests, component keyboard tests, Playwright settings flow, and native WDIO
registration/trigger tests.

### Meeting/device workflow

- Rust owns stable device identity, permission/preflight state, reconnect state,
  selected paths, recording sessions, job IDs, and durable progress.
- React renders typed snapshots/events and cannot manufacture authoritative job
  success, artifact paths, or recovery state.
- Mic-only and mic-plus-system sessions retain independent source tracks and
  exact gaps. Mixing is a derived playback/export, never the only evidence.
- Imported audio gets an admitted canonical path, cancellation semantics,
  resumable durable job state, and atomic artifact publication.

Required evidence: device hot-plug tests, real Windows loopback tests, supported
load with zero callback drops, crash/restart recovery at every transition,
malicious-path IPC tests, and fixture-based artifact/timeline checks.

## License and provenance gate

Studying these repositories does not change Yap's shipped notices. This audit
therefore does **not** add either source to `THIRD_PARTY_PROVENANCE.json`.

When source is adapted or copied, the same PR must:

1. Pin the donor repository and full commit.
2. Hash the exact donor license and every upstream file used.
3. Hash every resulting local file.
4. Add the applicable license and attribution to `THIRD_PARTY_NOTICES.md`.
5. For Apache-2.0 source, preserve required notices and prominently identify
   modifications; do not imply trademark permission.
6. Audit dependencies, fonts, icons, models, conversions, codecs, binaries, and
   downloaded assets separately. A root license is not evidence for them.
7. Update the release-contract provenance assertions and run them locally.

Branding, names, icons, screenshots, rendered system symbols, update keys,
installer bootstraps, FFmpeg downloads, and model artifacts are rejected unless
the user separately authorizes them after a complete provenance review.

## Execution order

1. Merge this research-only audit after its local documentation/provenance gate.
2. Inspect the connected GB10 node read-only and record its real network,
   runtime, GPU, storage, and service posture.
3. Implement the canonical Phase 3 server boundary in its own branch: OpenAPI,
   private capability health, capability-aware desktop connector, and Rust
   SQLite job ledger. Stop before auth, model pools, upload drain, or broad port
   exposure.
4. Implement the convergence client in isolated PRs: truthful one-window island
   and safe shortcuts first; meeting/device/import workflows second.
5. Prove one real vertical slice over the private connection, then profile and
   tighten it before adding broader server models, auth, diarization, or agents.

Every PR uses local/server evidence while hosted GitHub Actions are unavailable.
Hosted billing failures are infrastructure status, not passing or failing test
evidence.
