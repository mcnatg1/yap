# Architecture Checkpoint A Implementation Plan

**Status:** Implementation and documentation reconciliation complete; the final
exact-head gate remains pending after the merged Phase 5 baseline.

**Branch:** `refactor/phase1-5-architecture-checkpoint`

**Base:** `b6677631b2cc8283f0f6466622f2dfa7cfdb38f6`

**Scope:** Review and simplify the complete executable Phase 1–5 system without
adding Phase 6 product functionality.

## Governing outcomes

1. One explicit owner for every durable state and runtime lifecycle.
2. Correctness/security findings resolved before cleanup depends on them.
3. No duplicate UI/window/job/connector/result/retry/cancel authority.
4. Dead, superseded, speculative, and YAGNI machinery removed.
5. Mixed/oversized production and test surfaces decomposed or justified.
6. One-way dependency direction and bounded trust boundaries match code.
7. Efficiency claims supported by measurement or demonstrable work removal.
8. Current/normative/active/completed/historical/roadmap/runbook/evidence docs
   are distinguishable and linked.
9. One exact head passes the complete checkpoint gate once and merges through a
   focused reviewed PR.

## Review lenses

The repository-specific brief is the acceptance test. The following public
standards and practices are used only as additional review lenses, not as a
certification claim:

- [ISO/IEC 25010:2023](https://www.iso.org/standard/78176.html) for product
  quality characteristics;
- [ISO/IEC 5055:2021](https://www.iso.org/standard/80623.html) for automatable
  source-code quality measures;
- [ISO/IEC 25023:2016](https://www.iso.org/standard/35747.html) for quality
  measurement vocabulary;
- [NIST SSDF 1.1](https://csrc.nist.gov/pubs/sp/800/218/final) for secure
  development practice evidence; and
- [CMU SEI architecture evaluation](https://www.sei.cmu.edu/library/reduce-risk-with-architecture-evaluation/)
  for scenario- and risk-driven architecture review.

The concrete added checks are persisted-format compatibility, forbidden
dependency direction/cycles, deterministic builds and flake risk, redacted
failure observability, keyboard/focus/reduced-motion ownership, restart/cancel/
partial-publication/resource-exhaustion drills, and proof that complexity was
removed rather than moved.

## Completed slices

- [x] Independent read-only architecture/correctness and spec/provenance reviews.
- [x] Repository/module/test/doc/dependency/Git-object inventory.
- [x] Workflow, persistence, lifecycle, UI/window, network, and trust ownership map.
- [x] Correctness/security repairs required before decomposition.
- [x] Desktop job, recording, live runtime, connector, model, history, playback,
      app-state, and UI responsibility decomposition.
- [x] Server API/job/artifact/store/router/pool/worker responsibility decomposition.
- [x] Dedicated large test-harness partitioning.
- [x] Release contract one-way dependency repair and catch-all removal.
- [x] Exact attributed FreeFlow derivative set and pinned upstream/license closure.
- [x] Persisted connector configuration bounds and platform no-follow storage/
      lock handling with focused failure-path verification.
- [x] Bounded install identity, explicit server pool-contract dependency
      direction, and synchronous first-render reduced-motion preference.
- [x] Descriptor-bound server artifact validation plus shared bounded Rust and
      Python persisted-file readers.
- [x] Fixed-capacity shortcut/native-import dispatchers and exclusive admission
      for file-picker and server-settings workflows.
- [x] Findings register, ordered review slices, file-size inspection, and retained
      cohesion justifications.
- [x] Documentation classification, moved-reference repair, and current-state/
      ADR reconciliation.
- [x] Final focused diff review for correctness, security, accessibility,
      dependency direction, persisted compatibility, observability, and YAGNI.

The evidence is in [findings](../../evidence/architecture-checkpoint-a/FINDINGS.md),
[ownership](../../architecture/boundaries/PHASE-1-5-OWNERSHIP.md), and
[file inventory](../../evidence/architecture-checkpoint-a/FILE-INVENTORY.md).

## Remaining slices

- [ ] Freeze one exact head.
- [ ] Run the complete applicable local/native/server/GB10 checkpoint matrix
      exactly once and record immutable evidence.
- [ ] Open a focused PR; require hosted checks on the checked head, or disclose
      unavailable hosted checks with equivalent local evidence.
- [ ] Merge only after review and green exact-head evidence.

## One-time gate rule

Do not run the full matrix while implementation, provenance, docs, or review can
still change the candidate. Focused suites are allowed and required for the
surface being edited. Once frozen, the complete matrix must cover:

- frozen frontend install/build/unit/Playwright;
- Rust format, warnings-denied Clippy, library/integration/connector tests,
  dependency audit, and Windows dependency boundary;
- portable Python 3.12 server/contract/runtime/infra tests;
- release/provenance contracts and high-severity package audit;
- required hardware-independent native WDIO;
- applicable disposable-Windows installer contract/lifecycle boundary; and
- the GB10 Phase 5 vertical slice, restart/reconnect/cancel/resource bounds,
  immutable runtime/model identity, WER ceiling, and clean teardown.

Any executable change after that run invalidates the candidate and requires an
explicit gate decision. Documentation-only evidence correction must still be
reviewed and must not silently change the checked SHA claim.

## Prohibited scope

- Phase 6 preprocessing or new pipeline states;
- full Codex security plugin scans before the accepted Phase 10 enterprise gate;
- private scan artifacts or sensitive audio/transcript/host evidence in Git;
- developer-owned substitutes for IT-controlled identity/network/deployment;
- status or ADR score inflation before executable evidence; and
- broad dependency/tool upgrades unrelated to a checkpoint finding.
