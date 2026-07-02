# Yap documentation

| Document | Purpose |
|----------|---------|
| [**VOICE-OS-ARCHITECTURE.md**](VOICE-OS-ARCHITECTURE.md) | **Master spec** — high/low pipeline charts, 7 layers, coverage matrix, agents, failure states, roadmap |
| [**adr/README.md**](adr/README.md) | Architecture Decision Records (normative decisions) |
| [**specs/**](specs/) | Buildable implementation specs (IPC, error codes, lifecycle, acceptance) |
| [../PRODUCT.md](../PRODUCT.md) | Product purpose and UX principles |
| [../DESIGN.md](../DESIGN.md) | Visual and interaction design |

**Start here:** [VOICE-OS-ARCHITECTURE.md](VOICE-OS-ARCHITECTURE.md) for the full picture; ADRs for *why*; specs for *how to build*.

## Implementation specs

| Spec | Phase | Status |
|------|-------|--------|
| [phase-1-2-stt-sidecar.md](specs/phase-1-2-stt-sidecar.md) | 1–2 | Draft — local Moonshine fallback sidecar, IPC, errors, cutover |
| [phase-a-d-llm-sidecar.md](specs/phase-a-d-llm-sidecar.md) | A–D | Draft — llama-server, Polish migration, shared client |
| [phase-3-live-ux-audio.md](specs/phase-3-live-ux-audio.md) | 3 | Draft — mic, Silero audio thread, ghost UI, state map |
| [testing-strategy.md](specs/testing-strategy.md) | all | Draft — fixtures, WER gates, sidecar CI matrix |
