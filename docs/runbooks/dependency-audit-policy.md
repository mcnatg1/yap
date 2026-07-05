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

As of July 5, 2026, the CI command reports 17 allowed warnings and no unignored
vulnerability.

## Ignored Advisories

The CI ignore list is intentionally small:

- `RUSTSEC-2026-0194`
- `RUSTSEC-2026-0195`

Both are from `quick-xml` pulled transitively through `tauri-utils -> plist`.
Remove the ignores when `plist` can move to `quick-xml >= 0.41`.

## Change Rules

- New unignored vulnerabilities must fail CI.
- New ignores require a short justification and a removal condition in this
  runbook and `.github/workflows/ci.yml`.
- Dependency updates should prefer removing ignores over expanding the list.
