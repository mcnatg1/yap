# ADR 0023: Bounded live priority in the server workload router

**Date:** 2026-07-14
**Status:** Accepted
**Amends:** [ADR 0014](0014-server-tier-compute-topology.md) priority rule only

## Context

ADR 0014 gave interactive live ASR absolute priority over background batch
work. Absolute priority minimizes live latency, but an always-ready live queue
can prevent a ready imported recording from ever running. That violates the
same ADR's fairness and backpressure goals and makes a bounded batch queue a
place where accepted work can starve indefinitely.

Phase 4 introduces only an in-memory reference router. It has no authenticated
owner derivation, durable server queue, cancellation, recovery, service
integration, or production capacity result. Even at this limited boundary, its
scheduling rule should state the starvation tradeoff explicitly.

## Decision

Interactive live work remains preferred whenever live and batch work are both
ready. After a configurable bounded streak of live dispatches, the router must
dispatch one ready batch job before live preference resumes. The Phase 4
reference default is eight consecutive live dispatches.

The bound applies only when both targets are available and both queues contain
dispatchable work. It does not reserve an idle GPU slot, admit work beyond
queue limits, bypass owner round-robin fairness, derive identity, or authorize
a client/server transport surface.

This rule replaces only ADR 0014's statement that live work is always
prioritized. All other ADR 0014 topology, trust-boundary, fallback, and phase
decisions remain unchanged.

## Consequences

### Positive

- A continuous live workload cannot starve an already-ready batch job forever.
- Live work still receives the first dispatch and the large majority of
  dispatches under sustained mixed load.
- The maximum preference streak is executable and testable rather than an
  informal fairness claim.

### Negative

- One live dispatch can wait behind a forced batch dispatch after the bound is
  reached.
- The default of eight is a reference value, not a production latency or
  capacity result; production tuning requires measured mixed-workload data.

### Neutral

- Admission bounds, per-owner round robin, pool capacity, and backpressure stay
  independent of this priority rule.
- The Phase 5 candidate connects durable batch upload/drain to the router for
  one development owner. Live transport remains a later authenticated baseline,
  and Phase 7 still owns authenticated owner identity.

## Implementation notes

- `server/src/yap_server/workload_router/router.py` tracks the consecutive live
  dispatch streak and forces one ready batch dispatch at the configured bound.
- Focused router tests cover live preference, the bounded batch dispatch, owner
  round robin, target availability, duplicate rejection, and backpressure.
- The reference rule was included in the complete one-time Phase 4 matrix that
  passed on exact executable candidate
  `309a2d427707e3483b2649f13940bd48dfaee836`.
- The Phase 5 candidate routes loopback batch commits through this scheduler
  with one fixed development owner. It does not add a live target, authenticated
  ownership, measured mixed-load capacity, or production service integration;
  the complete Phase 5 gate is pending.
- Production promotion requires durable-queue, cancellation, recovery,
  authenticated-owner, and mixed-load latency/capacity evidence.

## Alternatives considered

- **Keep absolute live priority.** Rejected because an unbounded live stream can
  starve accepted batch work indefinitely.
- **Strict alternation.** Rejected because it gives background work too much
  influence over interactive latency when both queues stay busy.
- **Separate reserved workers now.** Deferred until multi-worker capacity is
  measured; Phase 4 proves only one transient worker.
