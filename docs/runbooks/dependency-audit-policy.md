# Dependency Audit Policy

Yap treats `cargo audit` vulnerabilities as release blockers unless the risk is
explicitly accepted in CI with a removal condition. Warnings are reviewed, but
they do not fail CI by themselves.

## Current Rust Policy

CI runs `cargo audit` for the Windows desktop target:

```powershell
cargo audit --target-os windows --target-arch x86_64
```

Warnings from Tauri's transitive target-all desktop stack are allowed while the
desktop app targets Windows first. The current warning set includes GTK3
bindings, `glib`, `proc-macro-error`, and `unic-*` crates pulled through that
graph. Do not add `cargo audit -D warnings` until the upstream Tauri dependency
graph no longer reports those warnings for crates we do not ship directly.

As of July 13, 2026, the CI command reports 17 allowed warning-class findings
and no vulnerability-class failure. Those warnings include the `glib`
unsoundness advisory described below.

## Open Target-Specific Alerts

GitHub Dependabot alert `GHSA-wrw7-89jp-8q8g` remains open for `glib` 0.18.5.
The advisory is medium severity, affects versions from 0.15.0 through 0.19.x,
and is patched in 0.20.0. The vulnerable crate is present in `Cargo.lock`
through Tauri's Linux GTK dependency path when Cargo resolves all targets.
CI enumerates the full locked Windows graph with `cargo tree --locked --offline
--target x86_64-pc-windows-msvc --prefix none --format "{p}"` and fails if any
package line starts with `glib v`. This broader boundary prevents a second
advisory-affected `glib` version from becoming Windows-reachable while 0.18.5
remains present only in the Linux graph. CI also fails if Cargo cannot complete
the graph inspection.

The Windows-scoped `cargo-audit` command still emits this lockfile advisory as
an allowed `unsound` warning. CI passes because warning-class findings are not
denied; the target distinction limits shipped exposure but is not what makes
the command exit successfully. This does not dismiss or close the alert. Keep
the GitHub alert open until the affected path is removed or upgraded. Enabling
Linux support, or changing the Tauri/GTK dependency graph, requires
reevaluating this alert before release and either removing the GTK path or
upgrading it to a graph that uses `glib` 0.20.0 or later.

## Ignored Advisories

The CI ignore list is empty. `RUSTSEC-2026-0194` and `RUSTSEC-2026-0195` were
removed after `plist` 1.10.0 moved the transitive parser to `quick-xml` 0.41.0.

## Change Rules

- New unignored vulnerabilities must fail CI.
- The Windows graph guard must reject every reachable `glib` version until the
  alert is removed or this policy is deliberately revised with new executable
  evidence.
- New ignores require a short justification and a removal condition in this
  runbook and `.github/workflows/ci.yml`.
- Dependency updates should prefer removing ignores over expanding the list.
- Linux support and Tauri/GTK dependency changes require a target-all audit and
  explicit reevaluation of every open target-specific alert.
