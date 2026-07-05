# Yap documentation

| Document | Purpose |
|----------|---------|
| [**VOICE-OS-ARCHITECTURE.md**](VOICE-OS-ARCHITECTURE.md) | **Master spec** — high/low pipeline charts, 7 layers, coverage matrix, agents, failure states, roadmap |
| [**adr/README.md**](adr/README.md) | Architecture Decision Records (normative decisions) |
| [**specs/**](specs/) | Buildable implementation specs (IPC, error codes, lifecycle, acceptance) |
| [**runbooks/**](runbooks/) | Operational setup notes for local/server environments |
| [../PRODUCT.md](../PRODUCT.md) | Product purpose and UX principles |
| [../DESIGN.md](../DESIGN.md) | Visual and interaction design |

**Start here:** [VOICE-OS-ARCHITECTURE.md](VOICE-OS-ARCHITECTURE.md) for the full picture; ADRs for *why*; specs for *how to build*. The canonical roadmap is client/server-shaped; some spec filenames still use historical phase labels.

## Implementation specs

| Spec | Roadmap area | Status |
|------|--------------|--------|
| [client-state-machine.md](specs/client-state-machine.md) | Phase 1/2 client workflow | Draft — recording-job state, setup/server axes, pipeline hooks |
| [phase-1-2-stt-sidecar.md](specs/phase-1-2-stt-sidecar.md) | Phase 2 fallback | Draft — local Moonshine fallback sidecar, IPC, errors, cutover |
| [phase-a-d-llm-sidecar.md](specs/phase-a-d-llm-sidecar.md) | Later polish | Draft — llama-server, Polish migration, shared client |
| [phase-3-live-ux-audio.md](specs/phase-3-live-ux-audio.md) | Phase 1/2 live UX | Draft — mic, Silero audio thread, ghost UI, state map |
| [phase-8-yap-server.md](specs/phase-8-yap-server.md) | Phase 3/4 server | Draft — staged monorepo server entrypoint, API contract, host setup |
| [testing-strategy.md](specs/testing-strategy.md) | all | Draft — fixtures, WER gates, sidecar CI matrix |

## Runbooks

| Runbook | Purpose |
|---------|---------|
| [dependency-audit-policy.md](runbooks/dependency-audit-policy.md) | Rust/Node audit expectations, ignored advisories, and warning policy |
| [yap-server-node-setup.md](runbooks/yap-server-node-setup.md) | Server node setup notes |
