# Yap documentation

| Document | Purpose |
|----------|---------|
| [**VOICE-OS-ARCHITECTURE.md**](VOICE-OS-ARCHITECTURE.md) | **Master spec** — high/low pipeline charts, 7 layers, coverage matrix, agents, failure states, roadmap |
| [**adr/README.md**](adr/README.md) | Architecture Decision Records (normative decisions) |
| [**specs/**](specs/) | Buildable implementation specs (IPC, error codes, lifecycle, acceptance) |
| [**runbooks/**](runbooks/) | Operational setup notes for local/server environments |
| [../PRODUCT.md](../PRODUCT.md) | Product purpose and UX principles |
| [../DESIGN.md](../DESIGN.md) | Visual and interaction design |

**Start here:** [VOICE-OS-ARCHITECTURE.md](VOICE-OS-ARCHITECTURE.md) for the full picture; ADRs for *why*; specs for *how to build*. The canonical roadmap is client/server-shaped.

## Implementation specs

| Spec | Roadmap area | Status |
|------|--------------|--------|
| [client-state-machine.md](specs/client-state-machine.md) | Desktop client workflow | Accepted contract; local/setup partial, server transitions unimplemented |
| [model-download-ux.md](specs/model-download-ux.md) | Local fallback setup | Implemented baseline; licensing, CI, and release gates remain |
| [local-audio-preprocessing-stack.md](specs/local-audio-preprocessing-stack.md) | Local audio preparation | Accepted design; production capture integration not implemented |
| [local-live-fallback-sidecar.md](specs/local-live-fallback-sidecar.md) | Local live fallback | Implemented baseline; CI, performance, and release gates remain |
| [local-llm-sidecar.md](specs/local-llm-sidecar.md) | Local LLM polish | Deferred draft; solo profile only |
| [live-dictation-client-ux.md](specs/live-dictation-client-ux.md) | Live dictation client | Implemented baseline; hardening remains |
| [server-tier-mvp.md](specs/server-tier-mvp.md) | Server tier MVP | Canonical Phase 3 draft; health/router skeleton only |
| [2026-07-10-source-aware-diarization-design.md](superpowers/specs/2026-07-10-source-aware-diarization-design.md) | Capture foundation and meeting evidence | Accepted design; foundation prerequisites are next, inference not implemented |
| [testing-strategy.md](specs/testing-strategy.md) | All | Living verification contract |

## Next implementation plans

Execute these in order:

1. [Client audio foundation](superpowers/plans/2026-07-10-client-audio-foundation.md) - canonical Phase 1 capture contracts, bounded sinks, crash-safe recording, and recovery.
2. [Server contract and durable connector](superpowers/plans/2026-07-10-server-contract-durable-connector.md) - canonical Phase 3 contract/health connector and the SQLite job-ledger prerequisite for Phase 5.

The second plan deliberately stops before upload drain, WSS runtime, server ASR, authentication, or diarization.

## Runbooks

| Runbook | Purpose |
|---------|---------|
| [dependency-audit-policy.md](runbooks/dependency-audit-policy.md) | Rust/Node audit expectations, ignored advisories, and warning policy |
| [repo-housekeeping.md](runbooks/repo-housekeeping.md) | Repo layout rules, naming conventions, and tech debt ledger |
| [yap-server-node-setup.md](runbooks/yap-server-node-setup.md) | Server node setup notes |
