# Changelog

This changelog records product/architecture milestones. Exact test counts and
immutable evidence belong in phase/checkpoint evidence records.

## Unreleased — Architecture Checkpoint A

- Reviewed the complete Phase 1–5 executable system for lifecycle, persistence,
  UI/window, network, trust, correctness, security, maintainability, and
  dependency ownership.
- Resolved checkpoint correctness/security findings in server worker cleanup,
  remote-origin cleanup, background lifecycle, playback source leases, model
  load integrity, legacy app-data migration, artifact publication, and release
  evidence dependencies.
- Bounded persisted configuration, install identity, desktop/server artifact
  reads, shortcut/drop work queues, and interactive picker/settings admission.
- Decomposed mixed desktop, server, test, and release-contract surfaces around
  explicit domain owners without adding Phase 6 behavior.
- Reconciled direct FreeFlow adaptations with the exact pinned MIT upstream and
  file-level provenance.
- Added current architecture/status/roadmap/security/provenance/evidence docs and
  separated active, completed, archived, and historical plans/designs.
- Full exact-head Checkpoint A gate and PR closure remain pending.

## 2026-07-15 — Phase 5 remote STT MVP

- Delivered durable canonical-WAV preparation, create/upload/commit/status/
  result/cancel, reconnect recovery, isolated private batch ASR, verified native
  result publication, and History projection through the loopback development
  contract.
- Passed the one-time local/native/server/GB10 gate and hosted checks on exact
  PR head `4771d9be60562fa009ccecbcd3c7111b699883a5`.
- Merged as `b6677631b2cc8283f0f6466622f2dfa7cfdb38f6`.

## 2026-07-14 — Phase 4 private ASR node

- Delivered the bounded router/pool, immutable model/runtime lock, and transient
  isolated Cohere batch worker on the pinned NVIDIA/Python 3.12 stack.
- Verified the reference worker on GB10 without installing a persistent service,
  opening an external listener, or changing host firewall policy.

## 2026-07-13 — Phase 3 server boundary

- Delivered machine-readable contracts, loopback capability health, connector
  generation/retry state, durable native job ledger, canonical app-data
  migration, stock NSIS closure, and disposable-Windows lifecycle proof.

## 2026-07-12 and earlier — Desktop foundation and local fallback

- Delivered tray/island ownership, deliberate shortcuts, bounded native capture,
  exact timeline loss, crash-safe recording/recovery, native history/playback
  admission, explicit local model lifecycle, and in-process Nemotron fallback.
