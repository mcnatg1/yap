# CI Actions And Cache Hardening Implementation Plan

> **Implementation status (2026-07-13):** Tasks 1-5 implemented and independently approved with 0 Critical/Important findings. Task 6 is active; keep public GitHub checks as the merge gate.

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:test-driven-development` for the release-contract changes and `superpowers:verification-before-completion` before opening or merging the PR.

**Goal:** Make public CI faster and safer without caching mutable build outputs, while preserving the exact release-evidence boundary when the repository later returns to private visibility.

**Architecture:** GitHub Actions may restore immutable dependency-download caches on every run, but only a successful trusted `main` push may save them. Pull requests, release builds, and NSIS smoke jobs are restore-only. Every third-party action is pinned to a reviewed commit. `cargo-audit` is downloaded from the official RustSec release and verified by SHA-256 instead of compiled from source on each run.

**Measured baseline:** The repository has 28 caches totaling 9.45 GiB, including three historical Cargo caches around 2.24 GiB each. PR-scoped duplicates consume most of the quota. Rust run `29193327424` spent 10m18s compiling `cargo-audit`; raw Cargo `target` directories range from 12.6 to 76.5 GiB locally and must not be cached.

**Tech Stack:** GitHub Actions, PowerShell 7.4, Node 24, pnpm 11.7, Rust/Cargo, Node's built-in test runner, YAML.

---

## Files

- Modify `.github/workflows/ci.yml`: exact action pins, restore-only PR caches, trusted-main saves, verified `cargo-audit` binary.
- Modify `.github/workflows/nsis-smoke.yml`: exact action upgrades and restore-only dependency caches.
- Modify `.github/workflows/release.yml`: exact action upgrades and restore-only dependency caches.
- Modify `.github/dependabot.yml`: group the aligned WDIO family and defer only broken cpal 0.18.0/0.18.1.
- Modify `desktop/tests/scripts/release-evidence.contract.mjs`: executable cache/action/audit-tool policy.
- Modify `docs/runbooks/dependency-audit-policy.md`: record the target-specific open glib alert and removal trigger.

## Non-Negotiable Constraints

- Do not cache Cargo `target`, `node_modules`, bundles, installers, models, recordings, transcripts, SQLite ledgers, test results, release seals, or release evidence.
- Do not cache credentials, certificates, tokens, `.env` files, or the RustSec advisory database.
- Treat every cache byte as readable by an untrusted fork pull request.
- Do not use broad `restore-keys`; keys must bind OS, architecture, and the relevant lockfile hash.
- Do not let pull requests, Dependabot, release, or scheduled NSIS jobs save caches.
- Set setup-node's `package-manager-cache: false` everywhere and do not use native cache inputs on setup-node, setup-python, or pnpm/action-setup; all cache writes must pass the explicit trusted-main contract.
- Keep release and NSIS builds independent of cached build outputs.
- Do not dismiss the Linux/GTK glib alert merely because the shipped target is Windows.
- Fail closed if CI cannot prove that every glib version remains absent from the exact Windows dependency graph.

---

### Task 1: Make The Release Contract Reject The Current Unsafe Shape

**Files:**
- Modify `desktop/tests/scripts/release-evidence.contract.mjs`

- [x] Add exact reviewed action constants for checkout v7, pnpm setup v6, setup-node v6, setup-python v6, Rust toolchain, cache restore/save v6, upload-artifact v7.0.1, and download-artifact v7.
- [x] Require every `uses:` entry in all three workflows to end in a 40-character commit SHA.
- [x] Assert that the reviewed workflow inventory equals every `.github/workflows/*.yml` or `.yaml` file so a new workflow cannot bypass policy.
- [x] Reject monolithic `actions/cache`; accept only `actions/cache/restore` and `actions/cache/save`.
- [x] Require restore keys to contain `runner.os`, `runner.arch`, and the relevant `hashFiles(...)` lock hash, with no `restore-keys` prefix.
- [x] Require every save step to use the restore step's `cache-primary-key`, run only after success on a `push` to `refs/heads/main`, and skip cache hits.
- [x] Reject `continue-on-error` before a cache save and allow only further cache-save steps after the first save.
- [x] Reject forbidden paths including `target`, `node_modules`, `bundle`, `dist`, test results, release evidence, and environment/credential material.
- [x] Require CI to download cargo-audit 0.22.2 from the official RustSec release, verify SHA-256 `0a7316540862c13d954f648917ceacca593747baed6eec180fafa590be2710ab`, and contain no `cargo install cargo-audit` command.
- [x] Bind the security-sensitive PowerShell bodies to exact reviewed templates so variable reassignment or command reordering fails the contract.
- [x] Run `pnpm test:release-contract` and record the expected RED failures against the old workflows.

### Task 2: Pin And Upgrade Every Workflow Action

**Files:**
- Modify `.github/workflows/ci.yml`
- Modify `.github/workflows/nsis-smoke.yml`
- Modify `.github/workflows/release.yml`

- [x] Replace tag references with reviewed commit pins and accurate version comments.
- [x] Upgrade `pnpm/action-setup` to v6.0.9 at commit `0ebf47130e4866e96fce0953f49152a61190b271`.
- [x] Upgrade `actions/upload-artifact` to v7.0.1 at commit `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a`.
- [x] Pin setup-python v6 at commit `ece7cb06caefa5fff74198d8649806c4678c61a1` and reuse the existing reviewed pins for the remaining actions.
- [x] Keep workflow token permissions read-only except the already-scoped release publishing job.

### Task 3: Implement Restore-Only Pull Request Caches

**Files:**
- Modify `.github/workflows/ci.yml`
- Modify `.github/workflows/nsis-smoke.yml`
- Modify `.github/workflows/release.yml`

- [x] Restore the fixed pnpm 11 store `~\AppData\Local\pnpm\store\v11` with key `pnpm-store-v11-${os}-${arch}-${pnpm-lock-hash}`.
- [x] Restore the Playwright browser directory with key `playwright-v1-${os}-${arch}-${pnpm-lock-hash}`.
- [x] Restore only Cargo registry index/cache and git DB with key `cargo-deps-v1-${os}-${arch}-${cargo-lock-hash}`.
- [x] Let the frontend job save pnpm and Playwright caches only after a successful trusted `main` push.
- [x] Let the Rust job save the Cargo dependency cache only after a successful trusted `main` push.
- [x] Keep native WDIO, release, and NSIS jobs restore-only so parallel jobs cannot race to write the same cache and release workflows cannot create private cache state.
- [x] Leave Python uncached because the server skeleton has no third-party Python dependency lockfile.

### Task 4: Replace Source-Built cargo-audit

**Files:**
- Modify `.github/workflows/ci.yml`

- [x] Download `cargo-audit-x86_64-pc-windows-msvc-v0.22.2.zip` into `RUNNER_TEMP` from the official RustSec release URL.
- [x] Verify the archive with PowerShell `Get-FileHash -Algorithm SHA256` before extraction.
- [x] Extract to a versioned temporary directory and invoke the binary directly with the existing Windows target policy.
- [x] Do not add the executable or advisory database to a cache.
- [x] Enumerate the full locked Windows Cargo graph before the audit and fail if inspection errors or any package line identifies a `glib` version.

### Task 5: Reconcile Dependabot And Audit Policy

**Files:**
- Modify `.github/dependabot.yml`
- Modify `docs/runbooks/dependency-audit-policy.md`

- [x] Group `@wdio/cli`, `@wdio/local-runner`, `@wdio/mocha-framework`, `@wdio/spec-reporter`, and `webdriverio` as `wdio-core`, and group the independently versioned `@wdio/tauri-plugin` plus `@wdio/tauri-service` as `wdio-tauri`.
- [x] Ignore only cpal 0.18.0 and 0.18.1; allow the next published release to be proposed automatically.
- [x] Record open alert GHSA-wrw7-89jp-8q8g as a target-all Linux GTK/glib path that is absent from the Windows graph.
- [x] Contract the executable graph guard so warning-class audit behavior cannot silently hide a change in shipped exposure.
- [x] State that Linux support or a Tauri/GTK dependency change triggers removal or remediation; keep the GitHub alert open until then.

### Task 6: Verify, Merge, And Clean Operational State

#### Task 6a: Bind The Cache Path To pnpm's Effective Store

The first trusted-main run after PR #42 saved Playwright successfully but emitted
`Path Validation Error` for pnpm because the reviewed cache directory did not
exist on the hosted runner. The cache policy must bind pnpm's consumer path, not
assume its environment-specific default.

- [x] Add a RED release-contract assertion requiring every pnpm-cached job to bind exactly one reviewed store before restore.
- [x] Derive the Windows store from `LocalApplicationData`, prove it equals the literal cache path, and set `PNPM_CONFIG_STORE_DIR` for all later steps.
- [x] Verify `pnpm store path` accepts the binding before persisting it through `GITHUB_ENV`.
- [x] Re-run the full release contract (33/33).
- [ ] Pass every public PR gate.
- [ ] Require the next trusted-main run to save both canonical frontend caches before deleting obsolete entries.

- [x] Run the release-contract test until GREEN (20/20).
- [x] Run workflow YAML parsing, `git diff --check`, and focused dependency-policy checks.
- [x] Request an independent security/release-boundary review and resolve every Critical or Important finding (0 remaining).
- [ ] Push a focused PR and require frontend, Rust, server, native WDIO, CodeQL, and release-contract checks to pass.
- [ ] After merge, verify the post-merge `main` run creates the new canonical caches.
- [ ] Delete obsolete PR-scoped caches and historical 2.24-GiB Cargo caches only after replacement caches exist; record before/after storage.
- [ ] Configure `main` protection with zero required approvals (sole-collaborator safe), required checks, conversation resolution, linear history, and no force-push/deletion.
- [ ] Enable exact-SHA action enforcement after every workflow is pinned; keep workflow tokens read-only by default.
- [ ] Verify the repository can later return to private visibility without changing contracts, only runner/budget strategy.

## Completion Evidence

- Action versions and SHAs are traceable to upstream releases.
- PR/fork runs cannot write dependency caches.
- No cached path contains mutable product, user, credential, test-result, or release-evidence state.
- `cargo-audit` remains checksum verified, and CI separately proves the Windows graph boundary for the open glib advisory.
- Public CI and post-merge `main` are green.
- Cache storage is materially below the 10-GiB eviction threshold.
