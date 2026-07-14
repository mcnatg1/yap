# Yap documentation

| Document | Purpose |
|----------|---------|
| [**VOICE-OS-ARCHITECTURE.md**](VOICE-OS-ARCHITECTURE.md) | **Master spec** — high/low pipeline charts, 7 layers, coverage matrix, agents, failure states, roadmap |
| [**adr/README.md**](adr/README.md) | Architecture Decision Records (normative decisions) |
| [**ADR-IMPLEMENTATION-STATUS.md**](ADR-IMPLEMENTATION-STATUS.md) | Living client/server ownership and implementation audit for every ADR |
| [**specs/**](specs/) | Buildable implementation specs (IPC, error codes, lifecycle, acceptance) |
| [**research/**](research/) | Pinned, non-normative external-source audits and selective reuse decisions |
| [**runbooks/**](runbooks/) | Operational setup notes for local/server environments |
| [../PRODUCT.md](../PRODUCT.md) | Product purpose and UX principles |
| [../DESIGN.md](../DESIGN.md) | Visual and interaction design |

**Start here:** [VOICE-OS-ARCHITECTURE.md](VOICE-OS-ARCHITECTURE.md) for the full picture; ADRs for *why*; specs for *how to build*. The canonical roadmap is client/server-shaped.

**Canonical Phase 9 knowledge boundary:** [ADR 0022](adr/0022-google-okf-permission-safe-projections.md) pins Google OKF v0.1 as the curated file/Git format, requires a Postgres typed-relationship and pgvector retrieval baseline, and treats Neo4j as a benchmark-gated challenger. It is a roadmap decision, not authorization to install databases or create Phase 9 infrastructure before the identity and meeting-result prerequisites exist.

## Implementation specs

| Spec | Roadmap area | Status |
|------|--------------|--------|
| [client-state-machine.md](specs/client-state-machine.md) | Desktop client workflow | Phase 3 durable imported-job ownership implemented in Rust; React is a typed projection |
| [model-download-ux.md](specs/model-download-ux.md) | Local fallback setup | Implemented baseline; model licensing, hosted real-model/native CI, and production-release proof remain |
| [local-audio-preprocessing-stack.md](specs/local-audio-preprocessing-stack.md) | Local audio preparation | Capture/timeline/recording foundation implemented; Silero, meeting inference, and transport remain |
| [local-live-fallback-sidecar.md](specs/local-live-fallback-sidecar.md) | Local live fallback | Implemented baseline; model licensing, hosted real-model/native CI, performance, and production-release proof remain |
| [local-llm-sidecar.md](specs/local-llm-sidecar.md) | Local LLM polish | Deferred draft; solo profile only |
| [live-dictation-client-ux.md](specs/live-dictation-client-ux.md) | Live dictation client | Windows baseline implemented; visible-bounds island and safe shortcut-recorder/default UX remain |
| [server-tier-mvp.md](specs/server-tier-mvp.md) | Server tier MVP | Phase 3 boundary plus isolated Phase 4 Cohere reference pool implemented; connected transport/auth/persistent deployment deferred |
| [2026-07-10-source-aware-diarization-design.md](superpowers/specs/2026-07-10-source-aware-diarization-design.md) | Capture foundation and meeting evidence | Capture/contract prerequisites implemented; speaker inference and server reconciliation not implemented |
| [testing-strategy.md](specs/testing-strategy.md) | All | Living verification contract |

## Current execution boundary

The tooling-only PowerShell migration is implemented: repo-owned Windows scripts require PowerShell Core 7.4 or newer, executable selectors use `pwsh.exe`, and each Windows CI job validates its isolated runtime. A hash-pinned 7.4.17 compatibility lane parses all tracked scripts. This does not broaden the product architecture or start server inference.

The [Server contract and durable connector](superpowers/plans/2026-07-10-server-contract-durable-connector.md) plan is the landed canonical Phase 3 implementation record: machine-readable contract, capability health service, connector state/retry, and SQLite job ledger. The Phase 4 private-ASR implementation adds a separate bounded reference router/pool and transient Cohere GB10 worker. Phase 5 upload drain, WSS runtime, authentication, persistent service integration, and diarization remain gated; none is implied by Phase 3 health reachability or the isolated Phase 4 worker.

The [Phase 4 private ASR node implementation record](superpowers/plans/2026-07-13-phase4-private-asr-node.md) pins the Cohere/NGC/Python 3.12 choices, executable and process-safety seams, focused GB10 evidence, and the remaining clean-head gate. Its focused pre-gate timings are not release, long-recording, or concurrency evidence.

The one-time Phase 3 implementation gate passed against exact candidate `c3999b7b685dd668165d54b64d1af61e41adad05`. Its immutable GB10 archive (`be7f43d757821c3e74d0ae2809599f5a84b369115d24afce42fe6687b1bf12e1`) passed 50/50 ARM64/Python 3.12 checks, transient loopback health drove the command-line connector to `Ready`, a separate refusal invocation drove it to `Retrying`, and teardown left no Yap process or port-18765 listener. Implementation head `a721121315c7a4bf5510212196141f17e9b237bd` subsequently passed hosted CI run `29293287930` and disposable-Windows stock NSIS lifecycle run `29293291582`. This is not evidence for a persistent service, same-process native UI transition, upload, WSS, authentication, ASR, external listener, or firewall change.

The [client audio foundation](superpowers/plans/2026-07-10-client-audio-foundation.md) is a landed implementation record. Its unchecked future inference/transport gates are not evidence that the capture foundation is absent.

## Runbooks

| Runbook | Purpose |
|---------|---------|
| [dependency-audit-policy.md](runbooks/dependency-audit-policy.md) | Rust/Node audit expectations, ignored advisories, and warning policy |
| [repo-housekeeping.md](runbooks/repo-housekeeping.md) | Repo layout rules, naming conventions, and tech debt ledger |
| [yap-server-node-setup.md](runbooks/yap-server-node-setup.md) | Server node setup notes |
