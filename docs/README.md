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
| [client-state-machine.md](specs/client-state-machine.md) | Desktop client workflow | Draft — recording-job state, setup/server axes, pipeline hooks |
| [local-live-fallback-sidecar.md](specs/local-live-fallback-sidecar.md) | Local live fallback | Draft — local Moonshine fallback sidecar, IPC, errors, cutover |
| [local-llm-sidecar.md](specs/local-llm-sidecar.md) | Local LLM polish | Draft — llama-server, Polish migration, shared client |
| [live-dictation-client-ux.md](specs/live-dictation-client-ux.md) | Live dictation client | Draft — mic, Silero audio thread, overlay UI, state map |
| [server-tier-mvp.md](specs/server-tier-mvp.md) | Server tier MVP | Draft — staged monorepo server entrypoint, API contract, host setup |
| [testing-strategy.md](specs/testing-strategy.md) | all | Draft — fixtures, WER gates, sidecar CI matrix |

## Runbooks

| Runbook | Purpose |
|---------|---------|
| [dependency-audit-policy.md](runbooks/dependency-audit-policy.md) | Rust/Node audit expectations, ignored advisories, and warning policy |
| [repo-housekeeping.md](runbooks/repo-housekeeping.md) | Repo layout rules, naming conventions, and tech debt ledger |
| [yap-server-node-setup.md](runbooks/yap-server-node-setup.md) | Server node setup notes |
