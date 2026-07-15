# Yap

Yap is a private, desktop-first transcription system: a Tauri/React client with
an explicit local live fallback and a durable batch path to a private GPU
server.

Phases 1–5 are merged. The current branch is the post-MVP
**Architecture Checkpoint A**, which is simplifying and documenting the
executing foundation without adding Phase 6 features.

Start with [current status](docs/CURRENT-STATUS.md). It states what executes,
what is verified, what is still absent, and what happens next.

## Current product boundary

- One installed desktop app owns tray/window lifecycle, native capture,
  deliberate shortcuts, local Nemotron fallback, durable imported jobs,
  connector state, authorized paths, and transcript History.
- One tray-owned island window expands on hover; native code owns its exact
  bounds and visible hit region.
- Imported Phase 5 jobs admit canonical mono PCM16/16 kHz WAV, publish an
  immutable Yap-owned spool, and persist create/upload/commit/status/result/
  cancel progress in native SQLite.
- The development server binds to numeric loopback. A user-managed SSH forward
  can connect it to the private GB-class node; Yap does not create an external
  application endpoint.
- The private worker uses the digest-pinned NVIDIA PyTorch 26.06 base, Python
  3.12, the locked NVIDIA Torch/CUDA stack, and the pinned Cohere model/runtime
  contract.
- Result identity, hashes, paths, sizes, authority, and transcript bytes are
  verified natively before History presents completion.

WSS/live server transcription, general media conversion, production
authentication, persistent multi-user service, enterprise DNS/certificates/
firewall/ZPA, diarization, and knowledge/agent features are later gates—not
hidden current capabilities.

## Repository map

```text
desktop/     Tauri 2 + React desktop app and native/runtime tests
server/      Python 3.12 contract, durable batch service, router, and worker
infra/       Private server-node bootstrap and policy
docs/        Current architecture/status, ADRs, specs, plans, runbooks, evidence
```

Runtime data belongs under Tauri's canonical app-data directory. On Windows
that is `%APPDATA%\com.mcnatg1.yap`. The stock NSIS installer lifecycle is
tested only in a disposable Windows environment.

## Desktop development

Requirements: Node 24, pnpm 11.7.0, Rust 1.96, and PowerShell Core 7.4+ for
repo-owned Windows automation.

```powershell
cd C:\dev\cohere-transcribe-local\desktop
corepack pnpm@11.7.0 install --frozen-lockfile
pnpm test
pnpm build
pnpm tauri dev
```

See [desktop/README.md](desktop/README.md) for focused Playwright, WDIO, and
installer commands. Do not run the installer lifecycle in an everyday Windows
profile.

## Server development

The portable service supports Python `>=3.12,<3.13`.

```powershell
$env:PYTHONPATH = (Resolve-Path "server/src").Path
uv run --isolated --no-project --python 3.12 --with pytest pytest server/tests
```

See [server/README.md](server/README.md) and the
[server-node runbook](docs/runbooks/yap-server-node-setup.md). The GB10 gate is
an exact-head release boundary, not a routine local test.

## Canonical documentation

- [Current status](docs/CURRENT-STATUS.md)
- [Current architecture](docs/architecture/CURRENT-ARCHITECTURE.md)
- [Phase 1–5 ownership map](docs/architecture/boundaries/PHASE-1-5-OWNERSHIP.md)
- [Roadmap](docs/roadmap/ROADMAP.md)
- [ADR index and implementation status](docs/adr/README.md)
- [Public security posture](docs/security/SECURITY-POSTURE.md)
- [Third-party provenance](docs/provenance/THIRD-PARTY.md)
- [Checkpoint findings and verification](docs/evidence/architecture-checkpoint-a/FINDINGS.md)
- [Documentation index](docs/README.md)
- [Changelog](CHANGELOG.md)

Product and visual intent remain in [PRODUCT.md](PRODUCT.md) and
[DESIGN.md](DESIGN.md). If a historical plan conflicts with current code or a
canonical document, the executable system and accepted ADR/spec win.
