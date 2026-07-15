# Architecture Checkpoint A Verification

This record separates inherited merged-phase evidence, focused checkpoint
development evidence, and the not-yet-run one-time checkpoint gate.

## Inherited Phase 5 baseline

Phase 5 PR head `4771d9be60562fa009ccecbcd3c7111b699883a5` passed its
one-time matrix and merged through
[PR #58](https://github.com/mcnatg1/yap/pull/58) as
`b6677631b2cc8283f0f6466622f2dfa7cfdb38f6`.

Recorded local/native/server/GB10 evidence on that head:

- Node 24 / pnpm 11 frozen install, production build, and high-severity audit
  with no finding;
- 32/32 release-contract tests;
- 271 frontend unit tests and 23 Playwright tests;
- 165 portable Python 3.12 server tests;
- 719 Rust library tests plus integration suites, format, warnings-denied
  Clippy, and connector integration;
- required native WDIO with 13 assertions;
- Rust dependency audit with zero vulnerabilities and 17 documented
  target-all/non-Windows warnings;
- Windows `glib` dependency boundary remained absent/unreachable;
- GB10 Python 3.12.3, NVIDIA Torch
  `2.13.0a0+8145d630e8.nv26.06`, CUDA 13.3, BF16, compute capability 12.1,
  and fixture WER `0.0` against the `0.12` ceiling;
- explicit SSH-forward interruption projected retrying and resumed the same
  durable job at the unchanged numeric-loopback origin; and
- container/process/listener teardown was clean with no external application
  listener or firewall/service mutation.

Hosted checks on the same PR head passed:

- CI: frontend, Rust, server, and required hardware-independent native WDIO;
- CodeQL: Actions, JavaScript/TypeScript, Python, and Rust.

This evidence is historical Phase 5 proof. Checkpoint A does not rerun or
rewrite Phase 5 merely because code was decomposed behavior-preservingly.

## Focused checkpoint evidence

Focused tests were run alongside the affected ownership/decomposition slices.
The ordered commit review is in [FINDINGS.md](FINDINGS.md). Counts from older
slices are not restated unless they were captured as immutable phase evidence;
the final checkpoint matrix will revalidate the integrated head once.

The release/provenance closure at implementation anchor `64539a0` recorded:

```powershell
corepack pnpm@11.7.0 --dir desktop test:release-contract
# PASS: 33/33

node desktop/tests/scripts/assert-third-party-provenance.mjs `
  --require-reviewed --verify-upstream
# PASS
```

Additional focused checks on that slice passed:

- direct release CLI execution reached argument validation without an import
  cycle or unsettled top-level await;
- Node syntax checks for every new `.mjs` owner;
- the PowerShell runtime-selector contract scanned the extracted Windows
  installer contract owner rather than a six-line aggregator; and
- `git diff --check`.

Documentation-only work used focused link/reference, classification, Markdown
diff, and stale-claim checks. It did not trigger unrelated product suites.

Recorded documentation checks on the reorganized tree:

```text
relative Markdown links: BROKEN_COUNT=0
relative Markdown heading anchors: BROKEN_ANCHOR_COUNT=0
legacy working-note path references: 0
current Phase 5 candidate/pending and queued-checkpoint claims: 0
current-authority delivered-capability future-tense drift: 0
git diff --check: PASS
```

The link audit resolves every repository-local Markdown target relative to its
containing document and skips only external URLs, mail links, and same-document
anchors. Historical command/path references affected by the moves were updated
as well as rendered links.

## Gate state

**The complete Checkpoint A gate has not run.**

It will run exactly once only after:

1. implementation/provenance owners are stable;
2. canonical docs, ADR status, and historical classification are reconciled;
3. moved links and public/private evidence boundaries pass focused checks;
4. the final architecture/correctness/security/accessibility review has no
   unresolved merge blocker; and
5. one exact head is frozen.

The required matrix is defined in the active
[checkpoint plan](../../plans/active/2026-07-15-architecture-checkpoint-a.md).
After the run, record the exact SHA, commands, counts, environment/runtime
identities, GB10 observations, cleanup results, and hosted check URLs here.

No status or ADR score may imply checkpoint completion before that evidence
exists.
