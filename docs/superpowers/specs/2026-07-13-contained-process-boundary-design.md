# Contained Windows Process Boundary Design

**Status:** Lean contained-process MVP implemented and verified locally on 2026-07-13; hosted PR and merged-main verification pending

**Date:** 2026-07-13

**Scope:** Windows installer smoke process ownership; advanced deterministic CI-boundary hardening is deferred

**Target:** Installer, installed application, and uninstaller processes launched by the NSIS smoke harness

## Lean MVP Scope Amendment (2026-07-13)

This approved amendment supersedes every later statement that makes nonce evidence, exhaustive lifecycle testing, canonical runners, dual-runtime verification, a dedicated CI context, or governance work part of current MVP acceptance, including the unfinished obligations in Sections 6, 7, 11, 12, and 13. The older design remains below as architectural history and a post-MVP hardening backlog; its unchecked or unmet items are not evidence that the contained-process MVP is incomplete.

The implemented MVP boundary provides:

- one authoritative `ContainedProcessLease` for each installer, installed-application, and uninstaller launch across the six real smoke phases: install, application, default uninstall, reinstall, explicit uninstall, and cleanup uninstall;
- suspended process creation, Job assignment and membership verification, retained original process and Job handles, a thread handle retained through exactly one resume, and identity capture before child code can run;
- retained-handle root-exit observation plus Job-quiescence proof, with a fail-closed mutation gate that preserves installer-owned paths when cleanup is unproven;
- one argument encoder, a typed final NSIS `/D=<absolute path>` tail, case-insensitive environment override/removal, working-directory support, and separate stdout/stderr redirection;
- no PID, `StartTime`, or CIM reconstruction as lifecycle authority; CIM is limited to a bounded executable-path diagnostic after lease disposal; and
- five focused real-Windows lifecycle cases covering natural exit, timeout/termination, descendant containment, nested-Job containment, and the required stdout/stderr, environment, and working-directory behavior.

The following work is explicitly deferred until after the MVP is landed and the user says `proceed`:

- atomic nonce-bound child evidence and adversarial publication cases;
- the exhaustive Task 4 fault matrix, eight-way concurrency, and 512-launch PID-churn campaign;
- a canonical bounded harness/runner, artifact-redaction framework, and dual-PowerShell-runtime verification;
- a dedicated `Windows process harness` CI job, required context, or workflow wiring;
- cache, open-PR, ruleset, Actions-policy, evidence-only-PR, and other governance automation;
- general PowerShell 7 migration, UI convergence, broader server work, HTTP/3, and WebSockets.

Hosted PR and merged-main checks remain landing gates. They do not reactivate the deferred harness or governance scope.

## 1. Decision

Yap will replace the NSIS smoke harness's split process ownership with one Windows-specific contained-process boundary. The boundary will retain the original process handle returned by `CreateProcessW`, assign the process to a kill-on-close Job Object before execution, and own waiting, exit observation, termination, and disposal for the complete lease lifetime.

The redesign is deliberately bounded. It applies to the installer, installed application, and uninstaller because those processes share one safety invariant: the harness must prove which executable it launched, contain its descendants, enforce a deadline, and leave no residual process before touching installer-owned paths. It does not absorb WDIO, the Tauri service, the Python server, the NSIS build process, or unrelated command execution.

Child-published test evidence will use an atomic, nonce-bound protocol. It will corroborate the native launch identity; it will never control process lifetime. Static release-contract tests will stop executing Windows process integration. A separately named Windows process-harness CI job will run the integration suite under both the pinned minimum PowerShell runtime and the hosted current runtime.

## 2. Why This Boundary Is Necessary

The post-merge `main` failure on 2026-07-13 was not evidence that the launcher returned the wrong process. The test waited only for `membership.pid` to exist, then immediately cast its contents to an integer. `Set-Content` can make the file visible before its contents are complete; an empty read becomes integer `0`, producing the misleading failure `Contained process handle did not identify the launched target.` A deterministic fixture reproduced that exact failure by exposing an empty file before publishing the PID.

Fixing only that read would leave a broader ownership split:

- `Yap.NsisSmoke.KillOnCloseJob.StartProcess` creates the process suspended with native `CreateProcess`, assigns it to a Job Object, reacquires a managed `Process` by PID, and closes the original process handle.
- `Start-JobContainedProcess` returns a loose `{ Job, Process }` pair whose two disposable members can escape or be disposed independently.
- `Invoke-ProcessWithDeadline` combines Job state, managed process state, tracked PIDs, creation-time identities, CIM snapshots, timeout policy, cleanup policy, and evidence assembly.
- The installed application bypasses that path and uses `Start-ProcessWithEnvironment`, `StartTime`, PID-based liveness, CIM descendant discovery, and reconstructed tree termination.
- `build-nsis-test.ps1` contains a third launch, wait, timeout, and kill implementation.
- `release-evidence.contract.mjs` mixes static policy inspection with execution of the full PowerShell integration suite. Frontend, native WDIO, scheduled smoke, and release workflows therefore run unrelated process integration under misleading step or job names.

The correct seam is not a generic process framework. It is the one release-harness boundary where exact native identity, containment, deadlines, and cleanup must be inseparable.

## 3. Goals

1. Give installer, application, and uninstaller execution exactly one lifecycle owner.
2. Retain the authoritative process handle from creation until disposal.
3. Assign the root process to the Job Object before any application code runs.
4. Make descendant containment a Job Object concern instead of reconstructed PID-tree ownership.
5. Support the application's isolated environment and working directory without a second launcher.
6. Preserve NSIS's special final `/D=<absolute path>` command-line contract without accepting arbitrary raw command text.
7. Make launch failure, timeout, termination, and disposal deterministic and bounded.
8. Publish test evidence atomically and bind it to one run with an unpredictable nonce.
9. Separate static policy contracts from Windows process integration.
10. Preserve all existing installer identity, sentinel, reparse-point, exact-artifact, and deletion-scope safeguards.

## 4. Non-Goals

- No cross-platform process abstraction.
- No production runtime or Tauri process-manager change.
- No migration of WDIO or `@wdio/tauri-service` ownership.
- No migration of the temporary Python contract server.
- No migration of `build-nsis-test.ps1` in this slice. It can become a later consumer only after the three smoke consumers prove the boundary.
- No shell execution and no arbitrary raw command-line API.
- No replacement of Job Objects with CIM, WMI, process-name matching, window enumeration, or filesystem heuristics.
- No weakening of destructive-uninstall authorization or path validation.
- No unrelated CI cache, dependency, governance, or application behavior change.

The implementation plan sequences this design with the still-open operational closure from the separately approved `2026-07-13-ci-actions-cache-hardening.md` plan because both must be complete before Phase 3 can close. Those imported cache, full-SHA Actions-policy, and open-PR audit obligations are not consequences of this process-boundary design and do not expand its runtime scope. Only adding the new stable `Windows process harness` context to the existing protection set is boundary-specific governance work.

## 5. Architecture

The design has production and test paths with one-way dependencies:

```text
SmokeOrchestrator
    -> WindowsContainedProcessLauncher
        -> ContainedProcessLease
    -> ResidualProcessAudit (postcondition only)

ProcessHarnessIntegrationTests
    -> WindowsContainedProcessLauncher
    -> NativeEvidenceProtocol (test fixtures only)
```

### 5.1 `LaunchRequest`

`LaunchRequest` is immutable validated input. It describes what to launch but owns no operating-system resources.

Required fields:

- absolute executable path;
- ordered normal arguments;
- absolute and distinct stdout/stderr paths;
- optional absolute working directory;
- environment overrides and removals.

The launcher accepts arguments as data, never as a shell string. Every argument rejects embedded NUL. Normal arguments use one reviewed Windows quoting implementation with contract tests for empty values, whitespace, quotes, and trailing backslashes.

NSIS's install directory is not represented as an arbitrary raw tail. `LaunchRequest` has private construction with separate normal-launch and NSIS-installer factories. Only the sealed NSIS factory accepts a typed `NsisInstallDirectory` value; the normal factory has no raw-tail parameter. The path must be absolute, contain no quote, NUL, carriage return, or line feed, and becomes the final raw `/D=<path>` token. No ordinary caller can construct or mutate an unquoted command tail.

Environment construction is isolated in one tested Unicode environment-block builder. It reads the inherited block through `GetEnvironmentStringsW` so hidden drive-current-directory entries such as `=C:` are preserved, then merges in a dictionary keyed by `OrdinalIgnoreCase`, rejecting inherited names that differ only by case and applying caller overrides/removals case-insensitively. Callers cannot add or modify names beginning with `=`. Only after merging does the builder sort a copied entry list with `OrdinalIgnoreCase` plus `Ordinal` as a deterministic emission tie-break, emit the required double-NUL terminator, and keep the buffer pinned and alive through `CreateProcessW` with `CREATE_UNICODE_ENVIRONMENT`. Environment values are not copied into logs or evidence.

### 5.2 `WindowsContainedProcessLauncher`

The launcher performs resource acquisition and the atomic launch transition. It contains mechanics, not smoke-test policy.

Launch order:

1. Validate `LaunchRequest` completely before acquiring resources.
2. Create valid inheritable stdout, stderr, and null-stdin handles and set `STARTF_USESTDHANDLES` with all three.
3. Create a non-inheritable Job Object and set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
4. Build an explicit inherited-handle allowlist containing only those three standard handles with `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
5. Call `CreateProcessW` with the verified absolute executable as non-null `lpApplicationName`, null process/thread security attributes, `bInheritHandles=TRUE`, `CREATE_SUSPENDED`, `CREATE_NO_WINDOW`, `EXTENDED_STARTUPINFO_PRESENT`, and, when needed, `CREATE_UNICODE_ENVIRONMENT`. Supply a mutable command line of at most 32,767 characters whose quoted first token is `argv[0]`, whose normal arguments use the reviewed encoder, and whose typed NSIS `/D=<absolute path>` value is the literal final tail.
6. Assign the original `PROCESS_INFORMATION.hProcess` to the Job Object.
7. Verify Job membership using the original handle.
8. Capture immutable identity from the original handle: process ID, full native creation `FILETIME`, and resolved image path.
9. Resume the primary thread exactly once, require the previous suspend count to equal `1`, and close the thread handle. Any other count is a launch failure and enters bounded cleanup.
10. Explicitly release and verify every launch-only parent resource while the launcher still owns the process and Job.
11. Transfer the process and Job handles into one `ContainedProcessLease`.

No application code can run before successful Job assignment. No code reconstructs the root process with `Process.GetProcessById`.

The production OS adapter is a stateless singleton. Every fallible native wrapper captures `GetLastWin32Error` immediately on its calling thread and returns one immutable per-call value/success/error result; there is no mutable `LastError` property or shared call state that parallel launches can overwrite. Logical failures after successful native calls carry no stale native code.

Only the non-inheritable original process handle and non-inheritable Job handle transfer into the lease. Before constructing that lease, the launcher closes its parent copies of stdin/stdout/stderr, closes the primary thread handle, calls `DeleteProcThreadAttributeList`, and frees the attribute-list, inherited-handle-list, mutable command-line, and Unicode environment allocations according to their native ownership rules. If explicit launch-only release fails, the launcher still owns process and Job and enters bounded failure cleanup; no already-constructed lease is lost behind a throwing `finally`. None of these launch-only resources can escape through the lease. Child-inherited redirection handles remain valid in the child independently of the parent's closed copies.

If any stage fails, the launcher retains the primary error, terminates any created suspended process or Job membership, waits for bounded cleanup, releases every acquired handle/allocation, and appends cleanup failures without masking the primary failure. Cleanup is proven only when every launch-only resource released successfully and the created root/Job state satisfies the applicable exit/quiescence conditions. A close/release failure keeps proof false even when no process remains, and a failed launch never returns a partial lease.

### 5.3 `ContainedProcessLease`

The lease is the sole lifecycle owner. It uses `SafeHandle`-based ownership for the original process and Job handles. The primary thread handle never escapes launch.

Immutable public evidence:

- root process ID;
- native creation `FILETIME` without millisecond truncation;
- resolved executable path.

Successful lease construction proves that Job assignment and resume completed. Partial launch state is never represented by a lease; it appears only in typed launch-error diagnostics.

Supported operations:

- `WaitForRootExit(deadline)` for bounded natural root exit;
- query exit code only after the process handle is signaled;
- `WaitForQuiescence(deadline)` for bounded Job quiescence;
- `TerminateAndWait(exitCode, deadline)` for bounded Job termination and quiescence;
- idempotent no-throw disposal as a last-resort resource backstop.

Job active-process counts and PID enumeration are private implementation details. The public operations return immutable evidence reports; callers cannot poll the Job or rebuild tracking and cleanup policy from lower-level primitives.

Normal orchestration never relies on `Dispose` to prove cleanup. `TerminateAndWait` calls `TerminateJobObject`, waits for the retained root-process handle, and queries Job state until active-process count reaches zero within the supplied deadline. It records any bounded-cleanup error before disposal. `Dispose` then closes the process handle followed by the Job handle, without throwing, so cleanup cannot mask an earlier launch or smoke failure. SafeHandle finalization is only an emergency backstop. Because the Job uses kill-on-close and this harness never detaches launched processes, disposing an unexpectedly active lease still initiates asynchronous termination as a last resort; it does not prove quiescence. There is no `Detach`, `ReleaseOwnership`, or implicit conversion to a bare PID or managed `Process`.

The lease does not know installer phases, filesystem sentinels, product identity, or evidence-file formats. Those remain orchestrator concerns.

### 5.4 `SmokeOrchestrator`

The orchestrator owns workflow policy and evidence assembly, not native handles.

It is a thin coordinator, not a reusable process-manager class. It sequences typed phase operations and consumes typed lease results; command construction, environment building, native waits, Job queries, and evidence parsing remain behind their owning boundaries.

- Installer and uninstaller phases launch a lease, wait for root exit within their deadline, require the expected exit code, wait for Job quiescence, record evidence, then dispose.
- The application phase launches a lease with isolated environment overrides, proves the process survives the configured launch-probe window, records identity and Job membership, terminates the Job, waits for quiescence, then disposes.
- Failure cleanup operates only through the lease. It never reconstructs a tree by PID.
- After lease disposal, `Assert-NoProcessesUnderPath` may use CIM executable-path inspection as a residual postcondition. A residual is a hard failure, but CIM does not decide what to terminate.

If explicit cleanup cannot prove Job quiescence, the orchestrator fails closed, retains the diagnostic evidence and filesystem state, and does not delete, rename, or otherwise touch installer-owned paths. A later zero-result CIM audit may corroborate successful cleanup but cannot turn unproven Job cleanup into success.

`Start-ProcessWithEnvironment`, `Stop-ProcessTreeBounded`, and root `StartTime` identity leave the installer smoke control path once all three consumers migrate. Obsolete PID/CIM control helpers are deleted only after a usage search proves they have no remaining approved consumer.

## 6. Native Evidence Protocol

Child-published evidence exists only to test that the exact executable reached user code. It never authorizes termination or substitutes for the lease identity.

Each integration run creates a sentinel-owned directory whose name includes a fresh 128-bit nonce from the platform cryptographic random-number generator, not only the parent PID. Reusing an existing run directory is an error.

The parent generates a nonce and expected final path. The child receives the nonce and:

1. creates a sibling temporary file in the same directory;
2. writes a versioned JSON record containing the nonce, its positive PID, and its resolved process path;
3. flushes and closes the file;
4. atomically renames the temporary file within the same directory to the unique final path, with no overwrite and no copy/cross-volume fallback.

Destination preexistence is terminal. The parent waits with a monotonic bounded deadline for the final path. Once visible, the record must parse strictly, use the supported schema version, match the nonce, contain a positive in-range PID exactly equal to `lease.RootProcessId`, and have a canonical resolved process path equal to both the lease identity and expected executable. Malformed or mismatched final evidence is terminal; it is not retried as though partial publication were valid. Failure messages include expected and observed non-sensitive identity fields.

Adversarial evidence tests cover an empty temporary file, partial JSON, stale valid evidence with the wrong nonce, whitespace/BOM variations, zero/negative/overflowing PIDs, a paused writer before rename, parallel launches, and an unexpected executable path. The final path must remain invisible until the complete record is closed.

## 7. CI And Test Ownership

`release-evidence.contract.mjs` remains a policy contract. It may parse workflows and source configuration and execute hermetic source/configuration fixture helpers, but it must not launch the contained-process/NSIS integration suite or assert live Windows process-lifetime behavior. Policy assertions validate public interfaces and safety invariants, not internal implementation text such as a particular `WaitForExit` or `Kill` call.

One checked-in entrypoint, `desktop/tests/scripts/run-contained-process-harness.ps1`, owns the integration command, result schema, and result-directory layout. It requires an absolute `PowerShellExecutable`, a runtime label, and a sentinel-owned result root. All workflows call this entrypoint rather than embedding or copying the suite body.

The entrypoint's per-suite watchdog is runner-local and is not a fourth consumer of the installer/app/uninstaller boundary. It sets every suite's working directory to the canonical repository `desktop` directory, retains the exact managed `Process` object it starts, passes arguments through `ProcessStartInfo.ArgumentList`, and continuously drains redirected streams through bounded collectors that discard/count output beyond a fixed cap. Runtime expiry triggers one `Kill(entireProcessTree: true)`; retained-process exit and both collectors then share one monotonic cleanup deadline rather than receiving additive waits. It sanitizes each suite log, accumulates scalar results, and atomically publishes one final `result.json` after success or first failure. It never imports `nsis-smoke-helpers.psm1`, never reacquires a PID, and never authorizes installer-owned filesystem mutation. The explicit integration fixture, not this watchdog, creates the outer-Job condition used to test nested Job assignment.

CI gains a separately named required job: `Windows process harness`. It runs:

1. parser and load checks under the pinned PowerShell 7.4.17 runtime;
2. the focused contained-process integration suite under PowerShell 7.4.17;
3. the same focused suite under the hosted current supported PowerShell runtime;
4. deterministic cleanup and residual-process assertions.

Before installing the pinned runtime, the job captures the hosted current `pwsh.exe` absolute path. A named install/probe step has its own five-minute deadline, extracts 7.4.17 to a separate absolute path, and does not add that directory to `PATH`; timeout/mismatch leaves the pinned output absent but cannot skip the hosted-current lane. Each lane invokes its runtime by absolute path, passes that same path to nested child fixtures, and asserts `ProcessPath`, `PSEdition`, and the expected exact version. This prevents the compatibility lane from contaminating current-runtime resolution.

The job runs on every pull request and `main` push without path filtering or a conditional skip, sets `timeout-minutes: 25` in addition to per-process deadlines, records both exact PowerShell identities, and fails if either runtime lane fails. The ceiling exceeds the declared worst-case sum of both lane deadlines plus setup/publication margin, so the second lane, upload, and aggregate step cannot be skipped merely because the first lane consumed its budget. It does not retry a failed integration case. On failure the canonical entrypoint writes nonce-bound expected/observed identity and cleanup evidence to its result root; CI uploads only `artifact/result.json`, `artifact/*.stdout.log`, and `artifact/*.stderr.log` with seven-day retention and rejects directory-wide artifact globs. Environment blocks, command lines, user data, raw/private logs, other artifact files, and temporary nonce files are excluded.

The frontend job no longer owns PowerShell process integration. Native WDIO no longer runs NSIS helper integration indirectly through the release contract. Scheduled NSIS smoke and release workflows invoke the canonical entrypoint once under their captured absolute current runtime before executing installer smoke; they do not copy its PowerShell body into workflow YAML. Every caller passes a known sentinel-owned result root. The dedicated two-lane job uploads before its final aggregate under `if: always()` plus explicit lane-outcome failure checks; scheduled and release callers use `if: failure()`. Each preserves the same explicit redacted evidence/log allowlist for seven days. This is intentional repetition at distinct release boundaries, not hidden duplication inside a policy test.

Repository governance requires only stable, always-reported contexts:

- `frontend`
- `rust`
- `server`
- `Native WDIO smoke (required, no hardware)`
- `Windows process harness`
- `CodeQL`

The four default-setup `Analyze (actions)`, `Analyze (javascript-typescript)`, `Analyze (python)`, and `Analyze (rust)` jobs are verified whenever GitHub emits them, but they are not individually required contexts: GitHub legitimately omits those matrix jobs on current dependency-only pull requests and reports the stable `CodeQL` aggregate as neutral instead. Requiring absent matrix internals would strand Dependabot updates. GitHub treats a neutral required check as successful; see [Troubleshooting required status checks](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/collaborating-on-repositories-with-code-quality-features/troubleshooting-required-status-checks).

The redesign PR merges only after all six stable contexts are successful and every emitted CodeQL analysis is green. The default-branch run must then complete all five GitHub Actions contexts, and the CodeQL analyses API must return the exact non-empty main-ref category set `/language:actions`, `/language:javascript-typescript`, `/language:python`, and `/language:rust` for the exact merged SHA, all complete and error-free. Only afterward may governance add `Windows process harness` using its observed integration ID; the complete ruleset is read back and compared so no stable context is dropped and matrix-internal `Analyze (...)` jobs are not accidentally pinned.

## 8. Error And Security Model

Request factories reject invalid data with `ArgumentException` before any native resource is acquired. Contained-operation errors identify the failed stage (`redirect`, `create-job`, `create-process`, `assign-job`, `capture-identity`, `resume`, `wait`, `terminate`, or `dispose`) and retain the native error code only when a native API actually failed. They do not log the full environment, inherited secrets, or arbitrary command lines.

All waits use monotonic time and explicit upper bounds supplied by the orchestrator. Explicit termination is idempotent, and disposal is an idempotent no-throw backstop. Cleanup failures from explicit operations are accumulated as evidence and cannot convert a failed cleanup into success.

Security invariants:

- executable paths are absolute, resolved, and shell-free;
- the caller's existing exact-artifact hash and installer-identity checks remain mandatory;
- only stdin/stdout/stderr handles are inheritable;
- normal argument quoting has one tested implementation;
- NSIS `/D=` is modeled as validated data, not a general raw-command escape hatch;
- no breakaway permission is enabled on the Job;
- no process is terminated solely because its PID, name, window, or path resembles the target;
- native evidence contains no environment values, command text, credentials, user data, or transcript data;
- recursive deletion remains independently guarded by strict-child, sentinel, and reparse-point checks.

The contained harness supports Windows 10 x64 or later and may itself run inside an outer CI Job Object. It never requests breakaway. If the host rejects nested assignment, launch fails with the native assignment error and no fallback. Integration coverage runs the harness inside an outer Job, proves membership in the inner Job, and proves inner termination and quiescence.

## 9. Anti-Pattern Guardrails

| Anti-pattern | Design guardrail |
|---|---|
| God process manager | Launcher handles acquisition; lease handles lifetime; orchestrator handles policy; evidence protocol handles test IPC. |
| Dual or ambiguous ownership | One non-detachable lease owns both process and Job handles. |
| PID as authority | PID is evidence only; wait, exit, and termination use retained native handles. |
| CIM/WMI as lifecycle control | CIM is permitted only after disposal as a residual audit. |
| Retry-until-green testing | Atomic publication plus deterministic adversarial fixtures replaces probabilistic reruns. |
| Sleeps as synchronization | Readiness is a nonce-bound atomic record or native handle signal; sleeps appear only inside bounded polling backoff. |
| Arbitrary command strings | Typed arguments and a single validated NSIS install-directory field. |
| Test flags in the public runtime API | Launcher and lease expose no test flags or environment switches; fakes exist only in the focused test load path. |
| Giant embedded native block | Native C# moves from the PowerShell module into a focused source file loaded by the module. |
| Static tests executing integration | Static Node contracts and Windows integration have separate commands and CI ownership. |
| Implementation-text pinning | Contracts assert behavior and public safety shape, not exact internal lines. |
| Premature generalization | Windows-only, three approved consumers, no cross-platform or universal process interface. |
| Scope creep | WDIO, server, build tooling, and product runtime remain explicit non-goals. |
| Silent fallback | Unsupported launch or cleanup states fail closed; there is no PID/CIM fallback path. |

## 10. Trade-Offs And Consequences

| Dimension | Contained boundary assessment |
|---|---|
| Correctness | Stronger: identity and lifetime remain attached to the original native handle. |
| Safety | Stronger: assignment occurs before resume and kill-on-close remains a final backstop. |
| Testability | Stronger: native mechanics, orchestration, and test IPC have separate contracts. |
| Complexity | Moderate one-time increase for `CreateProcessW`, Unicode environment construction, and `SafeHandle` ownership. |
| Maintenance | Lower after migration because three lifecycle paths become one; native interop remains Windows-specific expertise. |
| CI cost | One small additional required job, offset by removing hidden duplicate integration runs from frontend and WDIO. |
| Migration risk | Moderate and bounded to release verification; behavior-first migration keeps old consumers until parity is proven. |

Positive consequences:

- Process containment can be reasoned about from one owner and one state machine.
- Failure evidence names the actual Windows process-harness boundary.
- Installer safety no longer depends on PID timing or CIM ancestry reconstruction.
- Future changes to environment isolation, deadlines, or redirection have one implementation and one focused test owner.

Accepted costs:

- The repository retains a small amount of Windows native interop code.
- The CI protection set grows by one explicit status context.
- Migration must temporarily maintain old and new implementations, but never for the same launched process.
- Reviewers must validate handle ownership, command-line construction, and environment-block behavior with the same care as installer deletion logic.

Revisit triggers:

- If a fourth in-repository consumer needs identical containment, evaluate it against this private boundary before expanding the API.
- If a supported Windows version cannot accept nested Job assignment, fail that platform explicitly rather than adding PID fallback.
- If PowerShell can no longer compile/load the focused native source deterministically, package the same private boundary as a reviewed helper binary; do not move ownership into the product runtime.

## 11. Verification Design

### Pure and contract tests

- Windows argument quoting: empty, spaces, embedded quotes, and trailing backslashes.
- NSIS install-directory validation and final-token placement.
- Unicode environment merge, removal, sorting, invalid names, and embedded NUL rejection.
- Launch state transitions and failure cleanup through an internal OS adapter whose production construction is fixed to the real native implementation; fakes are compiled or loaded only by the focused test suite.
- Lease idempotence and invalid operation ordering.
- Static workflow ownership: release contract does not spawn integration; the named CI job runs both PowerShell runtimes.
- Public-API and behavior contracts expose no ownership conversion to a managed `Process` or PID, prove retained-handle behavior through PID churn, and prove residual auditing occurs only after lease-controlled cleanup. Source-text matching is diagnostic only and is not a merge gate.

### Windows integration tests

- Natural root exit and exact exit-code capture.
- Long-running root survives the probe, then Job termination signals the retained handle.
- Root creates a descendant; the descendant remains in the Job and dies on termination.
- Harness runs inside an outer Job; the root and descendant remain in the inner Job and inner termination reaches quiescence without breakaway.
- Immediate root exit remains identifiable through the retained process handle.
- Redirected files are released after natural exit, timeout, launch failure, and disposal.
- Assignment/resume failure leaves no executed marker and no residual process.
- Exact environment override/removal behavior.
- Working-directory behavior.
- Atomic nonce evidence and every adversarial publication case from Section 6.
- Parallel independent leases have no cross-talk.
- PID churn after exit cannot make a recycled PID part of the original lease.
- Every case proves retained-handle root exit and Job quiescence. Cases that use a case-local executable or descendant additionally prove the executable-path residual audit is empty; the end-to-end smoke always audits the installed application root after lease-controlled cleanup.

### Verification cadence

During implementation, each scoped task runs its affected pure/contract tests plus the smallest relevant Windows integration cases. The entire contained-process harness runs once the boundary is complete. The release contract, both PowerShell runtime lanes, native WDIO, installer smoke appropriate to the host, and the full Phase 3 branch gate then run once before PR merge. Hosted PR and post-merge checks must be green before governance requires the new context.

## 12. Migration Boundary

Implementation will proceed behavior-first and keep the old path available only until each approved consumer migrates. There is no runtime feature flag, automatic fallback, or catch-and-retry through the old launcher. Each consumer switches atomically: the old helper may remain solely for consumers not yet migrated, and the migrated consumer has exactly one owner.

Migration sequence:

1. Characterize current installer, app, and uninstaller behavior and introduce deterministic evidence fixtures.
2. Extract the native source and implement the launch request, launcher, and lease without changing consumers.
3. Migrate installer/uninstaller deadline execution.
4. Add environment/working-directory support and migrate application execution.
5. Remove PID/`StartTime`/CIM lifecycle ownership from the smoke path.
6. Split static and integration test commands and create the named CI job.
7. Delete obsolete helpers after zero-usage proof.
8. Reconcile testing strategy, release contracts, governance check lists, and Task 6 evidence.

Each migration step must keep one clear owner. No adapter may create a state where both the old process object and the lease believe they own termination, and a failed contained launch must fail closed rather than invoke the legacy path.

## 13. Acceptance Criteria

The design is implemented only when executable evidence proves all of the following:

- installer, installed app, and uninstaller use one contained-process boundary;
- the lease retains the original `CreateProcessW` process handle until disposal;
- the root process is assigned to the Job before resume;
- no approved smoke consumer reconstructs root ownership through PID or `StartTime`;
- no approved smoke consumer uses CIM to select processes for termination;
- environment overrides and removals preserve current isolated-app behavior;
- NSIS `/D=` semantics and all installer safety checks remain intact;
- launch, timeout, cleanup, and disposal are bounded and leave no residual process;
- child evidence is atomic, nonce-bound, schema-validated, and non-authoritative;
- the release policy contract executes no contained-process/NSIS integration;
- a separately named required CI job passes under PowerShell 7.4 and the hosted current runtime;
- frontend and native WDIO no longer hide NSIS process-integration execution;
- focused tests, installer smoke, and the complete Phase 3 gate pass;
- documentation and repository governance match the new executable truth.

## 14. Rejected Alternatives

### Retry until the PID file parses

This fixes the reproduced symptom but preserves stale-file collisions, duplicated lifecycle ownership, misleading test naming, and PID/CIM authority. It is an acceptable diagnostic experiment, not the productized design.

### Atomic evidence only

Atomic nonce evidence is required, but alone it leaves installer and application cleanup under different owners. It does not satisfy the one-owner invariant.

### Complete Windows harness rewrite

Absorbing WDIO, the Tauri service, server helpers, and build processes would conflate unrelated external owners and create a generic framework. Those consumers do not share the same release-harness invariant and remain out of scope.

### Rust or Tauri process helper

Routing release verification through product runtime code would couple artifact verification to the artifact being verified. The harness remains independent and Windows-native.

### Managed `Process` plus after-launch Job assignment

Starting normally and assigning afterward permits application code or descendants to run before containment. The boundary must create suspended and assign before resume.

## 15. Follow-On Decision

After this boundary is stable and Step 3 closes, `build-nsis-test.ps1` may be evaluated as a fourth consumer. Migration requires measured simplification and the same behavioral contract; it is not automatic. If no additional consumer needs the boundary, the API remains private to the release harness rather than becoming a shared platform abstraction.

## 16. Implementation Evidence

### Implemented core

- `9d711ba test: verify contained Windows process lifecycles` added the focused real-Windows lifecycle and PowerShell ownership contracts, including captured RED failures before migration.
- `be73e5a refactor: make the NSIS smoke lease authoritative` migrated every approved smoke phase, removed split PID/managed-process/CIM lifecycle ownership, and centralized fail-closed filesystem mutation authorization.

### Local verification

The single lean local release gate completed on 2026-07-13:

- `pwsh -NoLogo -NoProfile -NonInteractive -File .\desktop\tests\scripts\windows-contained-process.contract.test.ps1` — passed.
- `pwsh -NoLogo -NoProfile -NonInteractive -File .\desktop\tests\scripts\windows-contained-process.integration.test.ps1` — passed all five real-Windows cases.
- `pwsh -NoLogo -NoProfile -NonInteractive -File .\desktop\tests\scripts\nsis-smoke-helpers.test.ps1` — passed.
- `corepack pnpm@11.7.0 --dir ./desktop test:release-contract` — 32/32 policy checks passed after removing a hidden second execution of the already-focused NSIS helper integration. The earlier 33-test run exposed and corrected two inventory/contract drifts, but its count included that duplicate live suite.
- `corepack pnpm@11.7.0 --dir ./desktop build:nsis:test` — passed.
- `corepack pnpm@11.7.0 --dir ./desktop test:nsis:local` — the first run exposed a PowerShell ETS environment-value boundary defect; a focused RED/GREEN contract fixed it, and the rerun passed. Its fail-closed path retained the test footprint until a lease-proven recovery cleanup completed.
- `corepack pnpm@11.7.0 --dir ./desktop test:nsis:test-delete` — passed.
- `git diff --check` — passed.

### Hosted verification

This document commit intentionally does not preclaim future hosted evidence. The focused PR, exact-head emitted checks, match-head merge, and exact merged-main checks remain landing gates whose exact identifiers belong in the PR and final report. No deferred CI-harness or repository-governance work is required for this MVP landing.
