# Architecture Checkpoint A Findings

**Scope:** the executable Phase 1–5 system on
`refactor/phase1-5-architecture-checkpoint`.

**Baseline:** merged Phase 5 commit
`b6677631b2cc8283f0f6466622f2dfa7cfdb38f6`.

**Implementation review anchors:** `64539a0` (`refactor(release): decompose
evidence ownership`) and `d4e482a` (`fix(connector): bound persisted
configuration`). Documentation commits between those anchors do not change the
reviewed product behavior.

**Checkpoint state:** implementation review and focused verification are in
progress. The one-time complete checkpoint gate has not run.

## Review method

The governing checkpoint brief was read directly from the attached request. It
ends partway through the proposed documentation tree, so this register applies
the complete requirements that are present without inventing missing text.

The primary review used:

- independent, read-only architecture/correctness and specification/provenance
  reviews;
- the current branch, executable module graph, focused tests, and persisted
  formats as truth;
- a tracked-file and Git-object inventory;
- explicit inspection of every hand-written production or test file at or
  above 250 lines;
- dependency, lifecycle, persistence, UI, network, and trust-boundary tracing;
  and
- focused verification after each behavior-preserving extraction.

The following senior-review lenses were added because they expose failure
modes not captured by line count alone:

- persisted-format compatibility and migration invariants;
- forbidden dependency direction and import cycles;
- deterministic build, cache, and flake risk;
- observable failure points without private-content logging;
- keyboard, focus, reduced-motion, and accessible ownership in shipped UI;
- restart, cancellation, partial-publication, and resource-exhaustion drills;
  and
- evidence that complexity was removed rather than moved into a new catch-all.

These are review lenses, not claims of certification against ISO, NIST, CISA,
or any other quality or security standard.

## Consolidated target architecture

The smallest coherent Phase 1–5 architecture is:

```text
React views and hooks
  -> typed Tauri invoke/event adapters
  -> native command adapters
  -> native lifecycle/domain owners
  -> native SQLite and atomic app-data artifacts

native remote-job owner
  -> bounded connector contract
  -> loopback-only development transport
  -> server HTTP request adapters
  -> server job lifecycle owner
  -> server durable state and private artifacts
  -> bounded router and isolated batch worker
```

Rules enforced by the reviewed implementation:

1. Commands, HTTP handlers, and React components adapt; they do not own durable
   business transitions.
2. Native SQLite and native app-data artifacts own desktop recording/job truth.
   React state and `localStorage` may retain presentation preferences or
   compatibility projections, never authoritative job/result state.
3. The server job service coordinates one transaction lifecycle; the store,
   artifact, upload, completion, router, and worker modules own their individual
   mechanisms below it.
4. Configuration generation and server origin bind every in-flight remote
   operation. A response from stale configuration cannot become current truth.
5. Path admission, identity revalidation, bounded reads, and atomic publication
   stay adjacent to the artifact boundary they protect.
6. Runtime resources have one start/stop owner and background tasks have an
   explicit join/cancellation path.
7. One tray-owned native island owns live-window geometry and hit regions. The
   WebView projects its state and does not create a second window authority.
8. The server remains loopback-only for the Phase 5 development path. Identity,
   TLS, DNS, ZPA, policy, and enterprise deployment remain explicit later
   handoffs.

The detailed workflow map is in
[Phase 1–5 ownership](../../architecture/boundaries/PHASE-1-5-OWNERSHIP.md).

## Resolved correctness and security findings

Private scan material is intentionally absent. This public register identifies
the affected control and executable evidence without scan IDs, private paths,
or exploit detail.

| Area | Resolution on this branch | Evidence surface |
| --- | --- | --- |
| Server worker cancellation | Worker/container cleanup is durable and cannot be skipped by losing an in-memory cancellation path. | `4cbac8a`; server cancellation/runtime tests |
| Remote cleanup origin | Cleanup and cancellation remain bound to the durable server origin instead of whichever configuration is current later. | `eb9cb87`; remote recovery/reconfiguration tests |
| Background lifecycle | App-owned connector and job tasks have explicit startup/shutdown ownership. | `49f9c42`; app and lifecycle tests |
| Playback source replacement | Native playback uses admitted source leases and revalidates identity before source mutation/replacement. | `cab46d4`; playback registry/replacement tests |
| Local model load | Model loading verifies the pinned artifact set at the load boundary, not only at download time. | `760a4f4`; fallback/Nemotron tests |
| Legacy app-data migration | Recognized legacy entries migrate into Tauri app data through bounded, non-following, conflict-aware recovery. Conflicts fail visibly without deleting source data. | `913288a`; legacy migration lifecycle/security tests |
| File publication | Atomic text, transcript, recording, result, and configuration publication have explicit identity/validation owners. | `ca252e0` through `f116149`; focused durability/security suites |
| Request and response boundaries | Desktop and server validate bounded contract data at the transport edge before domain mutation. | `7556cac`, `cf01d34`, `483a95b`; contract tests |
| Release evidence import cycle | The stable release facade, CLI adapter, contract policy, Git fixture, and process access now form a one-way dependency graph. Direct CLI execution no longer stalls on top-level await. | `64539a0`; 33/33 focused release-contract tests |
| Upstream provenance identity | The reviewed source is identified as `zachlatta/freeflow`; every current attributed local derivative is hashed and tied to the pinned MIT upstream. | `64539a0`; reviewed-upstream provenance contract |
| Persisted connector configuration | Settings, origin approval, publication snapshots, and lock files now use a shared 64 KiB bound and platform no-follow regular-file opens. URL admission rejects inputs above 2,048 bytes before parsing. Oversized or linked existing state fails closed without replacement or recovery-artifact leakage. | `d4e482a`; 39/39 focused connector-configuration tests |

No known correctness or security finding from the checkpoint reviews remains
accepted without either a resolution or an explicit later-phase handoff below.
The final gate may still discover a new finding; any such finding reopens this
register and invalidates checkpoint closure until resolved.

## Resolved architecture and maintainability findings

| Former coupling | Current owners | Result |
| --- | --- | --- |
| Remote command, preparation, upload, polling, cancellation, and retry logic | `jobs/commands`, `jobs/drain`, `jobs/ledger`, and `jobs/remote` | Durable ledger remains authoritative; scheduling, transport, storage, and command adaptation are one-way dependencies. |
| Server route parsing, job validation, storage, lifecycle, completion, and worker execution | `api/*`, `jobs/*`, `workload_router/*`, and `pools/*` | HTTP parsing no longer owns business state; the service coordinates explicit sub-owners. |
| App-wide React state and feature behavior | feature hooks plus `components/app/*` and panel/view modules | `App.tsx` composes feature owners; native job/history truth is projected, not duplicated. |
| Live island rendering, presentation timing, waveform, motion preference, and native-surface synchronization | `components/live/*` | Visual concerns are independently testable while the native window remains authoritative. |
| Recording stream, durability, recovery, deletion, history, and transcript publication | `audio/recording/*`, `live/recordings/*`, `commands/history/*`, and `file_actions/*` | Mutation and catalog ownership are explicit; unrelated test harnesses are split. |
| Capture callback, worker processing, timeline loss, coordinator, stream finalization, and runtime control | `audio/capture/*`, `audio/timeline/*`, `audio/coordinator/*`, and `live/runtime/*` | Callback work stays bounded; lifecycle and state owners no longer share a catch-all runtime file. |
| Connector configuration, core policy, desktop adapter, health client, state, and batch protocol | `server_connector/config/*`, `core.rs`, `desktop.rs`, `client.rs`, `state.rs`, and `batch/*` | Bounded/no-follow persisted-file I/O, atomic publication, validation, transport, and applied state have separate owners below stable policy/state interfaces. |
| Fallback model download, operation, progress, artifact integrity, and Nemotron lifecycle | `stt/model/*`, `stt/fallback_model/*`, and `stt/nemotron/*` | Download orchestration no longer owns model-specific lifecycle or integrity policy. |
| Release artifact contract, process execution, workflow policy, Git fixture, and cache policy | `release-artifact/*` and `release-contract/*` | The former large mixed contract and replacement `context.mjs` catch-all were decomposed around real owners. |

The branch also removes speculative runtime orchestration state (`a1459b1`)
instead of preserving a production-looking Phase 6 placeholder.

## Checkpoint closure work

These are closure controls, not accepted product defects:

| Item | State | Evidence or required closure |
| --- | --- | --- |
| Documentation truth | Complete | Canonical status/architecture/roadmap/security/provenance docs describe merged Phase 5 and active Checkpoint A; historical plans/designs are separated. |
| File-size evidence | Complete | The complete 250-line inspection inventory and every retained >350-line cohesion justification are in `FILE-INVENTORY.md`. |
| Link integrity | Complete | Repository-local relative Markdown link audit passes after history-preserving moves. |
| Reviewability | Complete | The ordered commit/slice index below separates product refactors, release/provenance closure, and documentation. |
| Final exact-head verification | Pending | Freeze the branch only after final review settles; run the complete applicable checkpoint matrix exactly once. |
| Hosted closure | Pending | Open a focused PR and require the checked head to be green. If hosted checks are unavailable, record equivalent local evidence and disclose the unavailable checks. |

## Ordered review slices

The branch is intentionally a post-MVP checkpoint and therefore touches many
files. Review it in these contiguous slices rather than as one undifferentiated
diff:

| Slice | Commit span | Review focus |
| --- | --- | --- |
| 1 | `4cbac8a^..b170b85` | Immediate lifecycle/correctness repairs; first desktop/server/job/UI ownership splits. |
| 2 | `7e4a39d^..0d336ee` | Playback/history/app state, server HTTP/worker, audio manifest/timeline/coordinator/recording decomposition. |
| 3 | `760a4f4^..d068677` | Model integrity, removal of speculative orchestration, storage/path authority, recording recovery/deletion, live capture/runtime boundaries. |
| 4 | `00c878e^..6e780c5` | Live model/ASR/stream lifecycle, connector configuration, STT model lifecycle, contract trust boundaries, shortcut/action ownership. |
| 5 | `4fb68f0^..1a59e29` | Final recording/playback/server lifecycle splits and dedicated scenario-test partitioning. |
| 6 | `64539a0^..64539a0` | Release-evidence dependency repair and exact third-party provenance reconciliation. |
| 7 | `1139f48^..b0f7fd4` | Canonical status/architecture, inventory, plan/spec archival, and link repair. |
| 8 | `d4e482a^..d4e482a` | Persisted connector configuration bounds, no-follow storage/lock handling, and focused failure-path tests. |
| 9 | subsequent evidence-only commits | Final review reconciliation and exact-head gate evidence; no new product behavior. |

Each span starts after the previous slice's reviewed endpoint. Reviewers can use
`git diff <span>` or walk the commits inside a slice when a behavior-preserving
extraction and its dedicated test partition are adjacent.

## Explicitly deferred roadmap and enterprise work

The following are not gaps to fill during this checkpoint:

- Phase 6 preprocessing features such as VAD-driven chunk manifests, language
  identification, forced alignment, and new pipeline states;
- authenticated identity/authorization, Entra/MSAL, a Yap API audience, and
  tenant-derived ownership;
- anonymous-speaker inference, named-speaker reconciliation, knowledge/agent
  features, and MCP product surfaces;
- persistent production service supervision, backup/restore, external
  application listeners, TLS certificates, internal DNS, firewall policy, ZPA
  publication, and enterprise deployment; and
- a full Codex security scan before the accepted Phase 10 enterprise gate.

Those requirements remain in the roadmap and ADRs. Developer-owned
infrastructure must not be described as satisfying an IT-controlled handoff.

## Closure rule

Checkpoint A may be described as complete only when:

1. this register has no unresolved correctness/security item;
2. the ownership map matches executable behavior;
3. all retained size exceptions are justified;
4. current docs and ADR status contain no stale completion claims;
5. documentation links pass focused verification;
6. the exact branch head passes the one-time complete checkpoint matrix; and
7. that exact head is reviewed and merged through a focused PR.
