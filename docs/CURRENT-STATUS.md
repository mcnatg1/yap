# Current Status

**As of:** 2026-07-15

**Current work:** Architecture Checkpoint A on
`refactor/phase1-5-architecture-checkpoint`.

**Merged product baseline:** Phase 5 merge
`b6677631b2cc8283f0f6466622f2dfa7cfdb38f6` from
[PR #58](https://github.com/mcnatg1/yap/pull/58).

This document is the canonical human-readable status summary. Executable code,
machine-readable contracts, focused tests, and observed runtime behavior win if
another document disagrees.

## Milestone status

| Milestone | Status | Executable truth |
| --- | --- | --- |
| Phase 0: architecture reset | Merged | Thin desktop + private server direction and staged monorepo are accepted. |
| Phase 1: desktop foundation | Merged | Tray-owned app, capture/timeline/recording durability, native history/playback admission, and imported-job projection seams execute. |
| Phase 2: local fallback | Merged | Explicit Nemotron INT8 model lifecycle and in-process local live transcription execute; the runtime never silently downloads models. |
| Phase 3: server boundary | Merged and gated | Machine contracts, loopback health/capabilities, connector state/retry, durable desktop job ledger, canonical app-data migration, stock NSIS, and disposable-Windows lifecycle proof exist. |
| Phase 4: private ASR node | Merged and gated | A bounded router/pool and transient isolated Cohere worker ran on GB10 using the pinned Python 3.12 / NVIDIA PyTorch 26.06 stack. This is reference-worker proof, not a production service. |
| Phase 5: remote STT | Merged and gated | Canonical WAV admission, immutable desktop spool, durable create/upload/commit/status/result/cancel, isolated private batch inference, verified native result publication, reconnect recovery, and History projection execute through the loopback development contract. |
| Checkpoint A | Active | Phase 1–5 ownership, correctness, security, decomposition, dependency direction, provenance, efficiency, and documentation are being reconciled. No Phase 6 functionality is allowed. |
| Phases 6–10 | Planned | Follow the accepted order in the [roadmap](roadmap/ROADMAP.md). Enterprise infrastructure remains an explicit IT/security handoff. |

## What executes now

- One installed Tauri desktop app owns tray/window lifecycle, native recording,
  local fallback, imported-job durability, connector state, path authorization,
  transcript publication, and History truth.
- One tray-owned, hover-expanding island window projects live state. Native code
  owns its exact bounds and interactive region; no invisible sensor window owns
  clicks.
- Physical shortcut enrollment records deliberate chords. Normal typing is not
  treated as enrollment, and completed text uses the accepted safe delivery
  behavior rather than speculative field injection.
- Local live fallback uses the pinned Nemotron 3.5 ASR Streaming 0.6B INT8
  bundle through in-process `sherpa-onnx` after explicit installation.
- Imported Phase 5 jobs admit only already-canonical mono PCM16/16 kHz WAV at
  this boundary, prepare immutable Yap-owned artifacts, and persist progress in
  native SQLite.
- The development server path binds to numeric loopback. The desktop reaches a
  private node through an explicitly managed SSH forward; Yap does not create
  or silently fail over that tunnel.
- The private server validates bounded create/upload/commit requests, persists
  job/chunk/result state, routes one bounded batch workload, runs the isolated
  worker, and publishes an immutable result.
- Native code verifies result identity, authority, hashes, paths, sizes, and
  transcript bytes before History can present completion.

The complete owner and trust-boundary map is
[Phase 1–5 ownership](architecture/boundaries/PHASE-1-5-OWNERSHIP.md).

## What is not claimed

- No WSS/live server transcription, general media conversion, production
  authentication, external application endpoint, persistent supervised
  multi-user service, or measured multi-worker capacity is shipped.
- No Entra/MSAL token validation, tenant-derived owner, purpose grant, internal
  DNS, enterprise certificate, ZPA policy, or production firewall rule exists.
- Phase 6 preprocessing, Phase 8 speaker inference, and Phase 9 knowledge/agent
  behavior have not been pulled into the checkpoint.
- The full Checkpoint A matrix has not run. Focused development evidence is
  recorded in [checkpoint verification](evidence/architecture-checkpoint-a/VERIFICATION.md).
- Private security scans, scan identifiers, host paths, and detailed private
  findings are not repository or PR material.

## Phase 5 checked-head evidence

Phase 5 PR head `4771d9be60562fa009ccecbcd3c7111b699883a5` passed the
one-time local/native/server/GB10 gate and was merged by
`b6677631b2cc8283f0f6466622f2dfa7cfdb38f6` on 2026-07-15.
Hosted frontend, Rust, server, required native WDIO, and CodeQL analyses for
Actions, JavaScript/TypeScript, Python, and Rust were green on the checked PR
head. Exact counts and environment observations remain in the completed
[Phase 5 implementation record](plans/completed/2026-07-14-phase5-remote-stt.md)
and [verification record](evidence/architecture-checkpoint-a/VERIFICATION.md).

## Active next steps

1. Finish canonical documentation and link reconciliation.
2. Review the final diff for ownership, accessibility, failure observability,
   persisted compatibility, and dependency direction.
3. Freeze one exact checkpoint head.
4. Run the complete applicable checkpoint matrix exactly once.
5. Open a focused PR; merge only that checked, green head.
6. Begin Phase 6 only after Checkpoint A is merged.
