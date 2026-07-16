# Yap Documentation

This index separates current truth, accepted decisions/contracts, active work,
completed evidence, historical rationale, operations, security, and provenance.

If documents disagree, use this priority:

1. executable code, machine-readable contracts, and observed runtime behavior;
2. accepted ADRs and current normative specs;
3. [current status](CURRENT-STATUS.md) and
   [current architecture](architecture/CURRENT-ARCHITECTURE.md);
4. the [long-term Voice OS architecture frame](VOICE-OS-ARCHITECTURE.md) for
   eventual-system intent, subject to accepted ADR precedence;
5. active plans;
6. completed implementation records;
7. archived plans and historical designs.

Unchecked boxes in a completed/archived plan are execution history, not current
backlog.

## Current truth

- [Current status](CURRENT-STATUS.md)
- [Current architecture](architecture/CURRENT-ARCHITECTURE.md)
- [Phase 1–5 ownership and trust boundaries](architecture/boundaries/PHASE-1-5-OWNERSHIP.md)
- [Roadmap](roadmap/ROADMAP.md)
- [Changelog](../CHANGELOG.md)

## Long-term architecture frame

- [Yap & Voice OS system architecture](VOICE-OS-ARCHITECTURE.md)

This is the first-class readable frame for the eventual Voice OS system. It is
not an archive and must not be silently redefined during cleanup. Executable
behavior, accepted ADRs, current architecture/status, and the ordered roadmap
still control implementation and completion claims.

## Decisions and normative contracts

- [ADR index](adr/README.md)
- [ADR implementation status](ADR-IMPLEMENTATION-STATUS.md)
- [Client state machine](specs/client-state-machine.md)
- [Live dictation client](specs/live-dictation-client-ux.md)
- [Local live fallback](specs/local-live-fallback-sidecar.md)
- [Model download UX](specs/model-download-ux.md)
- [Audio preprocessing contract](specs/local-audio-preprocessing-stack.md)
- [Source-aware diarization](specs/source-aware-diarization.md)
- [Server tier MVP](specs/server-tier-mvp.md)
- [Testing strategy](specs/testing-strategy.md)

The [local LLM sidecar](specs/local-llm-sidecar.md) is an explicitly deferred,
non-normative design draft. It is discoverable for future re-evaluation but is
not current architecture, an active plan, or permission to add a runtime.

`server/openapi/openapi.json` and `server/openapi/live-events.schema.json` are
the normative machine-readable wire contracts. A route in a contract is not an
implementation claim; dynamic server capabilities and executable tests decide
availability.

## Plans

### Active

- [Architecture Checkpoint A](plans/active/2026-07-15-architecture-checkpoint-a.md)
- [CI actions and cache hardening](plans/active/2026-07-13-ci-actions-cache-hardening.md)

### Completed implementation records

- [Local Nemotron live transcription](plans/completed/2026-07-05-local-nemotron-live-transcription.md)
- [Model download UX](plans/completed/2026-07-08-model-download-ux.md)
- [Phase 3 server contract and durable connector](plans/completed/2026-07-10-server-contract-durable-connector.md)
- [Phase 4 private ASR node](plans/completed/2026-07-13-phase4-private-asr-node.md)
- [Phase 5 remote STT](plans/completed/2026-07-14-phase5-remote-stt.md)

### Archived plans and historical designs

The [plans index](plans/README.md) defines lifecycle rules. Superseded recipes
live under [plans/archived](plans/archived/). Retired design snapshots live
under [archive/historical-designs](archive/historical-designs/).
They preserve rationale and provenance but are not current implementation
instructions. Detailed historical task reports live under
[archive/implementation-evidence](archive/implementation-evidence/); see the
[archive index](archive/README.md).

## Operations, research, security, and provenance

- [Server-node setup](runbooks/yap-server-node-setup.md)
- [Dependency audit policy](runbooks/dependency-audit-policy.md)
- [Repository housekeeping](runbooks/repo-housekeeping.md)
- [Research index](research/README.md)
- [Public security posture](security/SECURITY-POSTURE.md)
- [Third-party provenance](provenance/THIRD-PARTY.md)

## Verification evidence

- [Evidence policy and index](evidence/README.md)
- [Checkpoint A findings](evidence/architecture-checkpoint-a/FINDINGS.md)
- [Checkpoint A file inventory](evidence/architecture-checkpoint-a/FILE-INVENTORY.md)
- [Checkpoint A verification](evidence/architecture-checkpoint-a/VERIFICATION.md)

Private scans, scan identifiers, sensitive audio/transcript data, host paths,
and raw machine evidence do not belong in this documentation tree, PRs, CI
logs, or tracked test results.
