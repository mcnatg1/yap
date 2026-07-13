# Contained Windows Process Boundary Implementation Plan

> **Current status:** The lean MVP amendment below is the only operative implementation plan. The archived enterprise plan is retained for post-MVP reference and must not be resumed without an explicit `proceed`.

**Goal:** Replace the NSIS smoke harness's split PID, managed-process, Job Object, and CIM lifecycle ownership with one retained-handle Windows process lease and the essential real-Windows lifecycle evidence.

**Architecture:** A Windows-only C# boundary validates launch requests, creates the process suspended, assigns and verifies it in a kill-on-close Job Object, retains the original process and Job handles, resumes once, and returns one non-detachable lease. The PowerShell smoke script is a thin policy coordinator and authorizes installer-owned mutation only after explicit root-exit and Job-quiescence proof. CIM remains only a bounded post-disposal executable-path diagnostic.

**Tech Stack:** PowerShell Core 7.4+, C# compiled with `Add-Type`, Windows Job Objects and `CreateProcessW`, Node 24's built-in test runner, GitHub Actions, NSIS/Tauri release smoke.

## Lean MVP Scope Amendment (2026-07-13)

This approved amendment supersedes all unfinished execution work in Tasks 3-10 below. Tasks 0-2 remain completed history. Everything under **Archived Enterprise Plan (Non-Operative)** is retained only as a post-MVP hardening backlog and design record; its constraints, file inventory, commands, checkboxes, completion definition, dedicated CI context, and governance steps are not current execution instructions.

The lean closure delivered one authoritative retained-handle `ContainedProcessLease` across the six actual NSIS smoke phases. It preserves suspended creation, assign-and-verify before one resume, original-handle ownership, root-exit and Job-quiescence proof, fail-closed filesystem mutation, typed NSIS `/D=`, argument/environment/working-directory/stream behavior, and diagnostic-only post-disposal CIM use. Five real-Windows cases cover the essential lifecycle boundary.

Deferred work includes atomic nonce evidence, exhaustive fault/concurrency/PID-churn matrices, the canonical bounded runner and redaction framework, dual-runtime verification, a dedicated `Windows process harness` workflow/context, cache/open-PR/ruleset/Actions-policy/evidence-PR governance, general PowerShell 7 migration, UI convergence, broader server work, HTTP/3, and WebSockets.

### Active MVP constraints and files

- The lease remains the sole lifecycle authority for installer, application, and uninstaller launches; PID, `StartTime`, managed `Process`, and CIM cannot select or terminate them.
- Every child is created suspended, assigned and verified before one resume, and represented by retained original process and Job handles until lease disposal.
- Caught exceptions, disposal, missing PIDs, or a clean CIM diagnostic never restore mutation authority.
- Only the three focused PowerShell contracts and the one lean local release gate are in scope.
- Active code and test files are `windows-contained-process.cs`, `windows-contained-process.contract.test.ps1`, `windows-contained-process-fixture.ps1`, `windows-contained-process.integration.test.ps1`, `nsis-smoke-helpers.psm1`, `nsis-smoke-helpers.test.ps1`, `smoke-nsis.ps1`, and `release-evidence.contract.mjs` under `desktop/tests/scripts/`, plus this plan and the paired design.

### Lean MVP closure evidence

- [x] Preserve completed Tasks 0-2 and their reviewed public contracts.
- [x] Capture focused RED evidence before migrating the real smoke consumers.
- [x] Migrate install, application, default uninstall, reinstall, explicit uninstall, and cleanup uninstall to one lease owner.
- [x] Remove PID/`StartTime`/managed-`Process`/CIM lifecycle ownership from the smoke path.
- [x] Gate every installer-owned filesystem mutation on lease-proven root exit and Job quiescence.
- [x] Pass the focused contract, five-case integration, helper, release-contract, NSIS build, local smoke, delete smoke, and diff checks.
- [x] Reconcile the design and plan with the implemented MVP and explicit deferrals.

The local implementation commits are `9d711ba test: verify contained Windows process lifecycles` and `be73e5a refactor: make the NSIS smoke lease authoritative`. Direct/final review, exact-head PR checks, match-head merge, and exact-main verification remain mandatory landing gates, but their evidence belongs in the PR and final report rather than being preclaimed by this document commit.

---

## Archived Enterprise Plan (Non-Operative)

The remainder of this document records the superseded enterprise implementation sequence. Proposed files may not exist, expected test counts may describe work never implemented, and every instruction involving nonce evidence, exhaustive lifecycle matrices, canonical runners, redaction, dual runtimes, dedicated workflow contexts, or repository governance is deferred. Do not execute this remainder until the user explicitly says `proceed` and approves a new plan.

### Historical Global Constraints

- Support Windows 10 x64 or later; do not introduce a cross-platform process abstraction.
- The minimum PowerShell runtime is Core 7.4; CI must exercise exact PowerShell 7.4.17 and the hosted current runtime.
- Apply the boundary only to installer, installed application, and uninstaller processes launched by `smoke-nsis.ps1`.
- Do not migrate WDIO, `@wdio/tauri-service`, the Python server, `build-nsis-test.ps1`, or product/Tauri runtime process ownership.
- Retain the original `CreateProcessW` process handle and Job handle until lease disposal; never reconstruct root ownership from PID or `StartTime`.
- Create suspended, assign and verify Job membership, capture identity, then resume exactly once.
- Expose no raw Job queries, managed `Process`, detach operation, PID-directed termination, public test flag, environment test switch, or catch-and-retry legacy fallback.
- Normal arguments use one Windows encoder. NSIS `/D={absolute install directory}` is a typed final tail, never a general raw-command field.
- Preserve hidden environment entries such as `=C:`, merge names case-insensitively, and pass one pinned double-NUL Unicode block to `CreateProcessW`.
- Treat `Dispose` as an idempotent no-throw emergency backstop; explicit cleanup must prove root exit and Job quiescence before installer-owned paths are touched.
- Child evidence is test-only, schema-versioned, 128-bit nonce-bound, atomically published in one directory, and never lifecycle authority.
- Do not retry failed integration cases. Use deterministic fixtures and bounded waits.
- During each task, run only the affected contract tests and smallest relevant Windows integration cases. Run the complete Phase 3 gate once after the boundary is complete.
- Keep workflow actions at their reviewed full commit SHAs and workflow tokens read-only by default.
- Required repository contexts are the six stable names `frontend`, `rust`, `server`, `Native WDIO smoke (required, no hardware)`, `Windows process harness`, and `CodeQL`. Audit emitted `Analyze (...)` jobs without requiring matrix-internal names.

---

### Historical Scope And File Map

This is a proposed target inventory, not a description of the lean MVP worktree.

### New focused files

- `desktop/tests/scripts/windows-contained-process.cs` — production-only launch request, native adapter, launcher, safe handles, lease, immutable reports, and typed errors.
- `desktop/tests/scripts/windows-contained-process.testing.cs` — test-load-only probes and fault-injecting native adapter; never loaded by release scripts or product workflows.
- `desktop/tests/scripts/windows-contained-process.contract.test.ps1` — request, quoting, environment, state-machine, and failure-cleanup contract tests.
- `desktop/tests/scripts/contained-process-evidence.psm1` — test-only nonce creation, bounded final-record wait, strict parse, and identity validation.
- `desktop/tests/scripts/contained-process-evidence-child.ps1` — test child that flushes and atomically renames one complete identity record.
- `desktop/tests/scripts/contained-process-evidence.test.ps1` — adversarial evidence publication tests.
- `desktop/tests/scripts/windows-contained-process-fixture.ps1` — test-only descendant and atomic nested-Job result fixture.
- `desktop/tests/scripts/windows-contained-process.integration.test.ps1` — real Windows natural-exit, timeout, descendant, nested-Job, redirection, environment, working-directory, parallel, and PID-churn cases.
- `desktop/tests/scripts/contained-process-redaction.psm1` — pure path-identity and bounded artifact-log sanitization.
- `desktop/tests/scripts/contained-process-redaction.test.ps1` — adversarial public-artifact redaction contracts.
- `desktop/tests/scripts/contained-process-harness-runner.psm1` — runner-only retained-managed-process deadline and asynchronous stream-drain helper.
- `desktop/tests/scripts/contained-process-harness-runner.test.ps1` — normal, high-output, timeout, descendant-kill, and bounded-cleanup runner contracts.
- `desktop/tests/scripts/run-contained-process-harness.ps1` — canonical runtime-identity, parser, suite-selection, log, and redacted-result entrypoint.

### Existing files changed

- `desktop/tests/scripts/nsis-smoke-helpers.psm1` — load production C# and expose thin typed launch adapters; retain filesystem and residual-audit helpers.
- `desktop/tests/scripts/nsis-smoke-helpers.test.ps1` — retain path, hash, reparse-point, sentinel, quarantine, and run-lock tests; move live process tests to the focused suite.
- `desktop/tests/scripts/smoke-nsis.ps1` — migrate all installer/app/uninstaller phases to leases and fail closed on unproven cleanup.
- `desktop/tests/scripts/release-evidence.contract.mjs` — remain a policy contract and stop launching process integration.
- `desktop/package.json` — verify unchanged: `test:release-contract` remains policy-only and no PATH-dependent alias hides the canonical harness.
- `.github/workflows/ci.yml` — add the named process-harness job and remove process integration from frontend.
- `.github/workflows/nsis-smoke.yml` — invoke the canonical harness once under captured current PowerShell before installer smoke.
- `.github/workflows/release.yml` — invoke the canonical harness once before the sealed installer smoke and retain redacted failure evidence.
- `docs/specs/testing-strategy.md`, `README.md`, `docs/README.md`, `docs/ADR-IMPLEMENTATION-STATUS.md` — reconcile documented ownership and executable evidence.
- `docs/superpowers/plans/2026-07-13-ci-actions-cache-hardening.md` — reconcile Task 6 with PR #52, the failed post-main run, canonical caches, the replacement PR, and final governance.

---

### Task 0: Rebase And Name The Implementation Branch

**Files:** None.

- [ ] **Step 1: Confirm the isolated worktree is clean and fetch `main`**

Run from `C:\dev\cohere-transcribe-local\.worktrees\contained-process-boundary-design` after this plan is committed and an execution mode is chosen:

```powershell
git status --short
git fetch origin main
git log --oneline --decorate HEAD..origin/main
```

Expected: the first command prints nothing. Inspect every newer `main` commit before rebasing; do not overwrite unrelated work.

- [ ] **Step 2: Rebase the documentation commits and rename the branch**

```powershell
git rebase origin/main
git branch -m fix/contained-windows-process-boundary
git status --short
```

Expected: rebase succeeds without dropping either the approved design or this implementation plan, the branch is `fix/contained-windows-process-boundary`, and the tree is clean. Resolve any documentation-only conflict deliberately; stop if a product-code conflict appears because that means the implementation boundary changed underneath the plan.

---

### Task 1: Build Immutable Launch Requests And Pure Contracts

**Files:**
- Create: `desktop/tests/scripts/windows-contained-process.cs`
- Create: `desktop/tests/scripts/windows-contained-process.testing.cs`
- Create: `desktop/tests/scripts/windows-contained-process.contract.test.ps1`
- Modify: `desktop/tests/scripts/nsis-smoke-helpers.psm1:1-12`

**Interfaces:**
- Produces: `Yap.NsisSmoke.LaunchRequest.Create(...)` for normal arguments.
- Produces: `Yap.NsisSmoke.LaunchRequest.CreateNsisInstaller(...)` for a typed final `/D=` value.
- Produces: `Yap.NsisSmoke.NsisInstallDirectory.Create(string)`.
- Produces: immutable `ExecutablePath`, `Arguments`, `StdoutPath`, `StderrPath`, `WorkingDirectory`, `EnvironmentOverrides`, and `EnvironmentRemovals` properties.
- Produces for tests only: `Yap.NsisSmoke.Testing.LaunchRequestProbe.BuildCommandLine(...)` and `BuildEnvironmentBlockText(...)`.

- [ ] **Step 1: Write the failing request contract**

Create `windows-contained-process.contract.test.ps1` with `#requires -Version 7.4`, `#requires -PSEdition Core`, strict mode, a local `Assert-True`, and these exact cases:

```powershell
$productionSource = Join-Path $PSScriptRoot "windows-contained-process.cs"
$testingSource = Join-Path $PSScriptRoot "windows-contained-process.testing.cs"
Add-Type -Path @($productionSource, $testingSource)

$runtime = [IO.Path]::GetFullPath([Environment]::ProcessPath)
$nonce = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(16)).ToLowerInvariant()
$root = [IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) "yap-launch-request-$nonce"))
New-Item -ItemType Directory -Path $root -ErrorAction Stop | Out-Null
$sentinel = Join-Path $root ".yap-launch-request-v1"
$sentinelStream = [IO.FileStream]::new($sentinel, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
$sentinelStream.Dispose()
$stdout = Join-Path $root "stdout.log"
$stderr = Join-Path $root "stderr.log"

$arguments = [string[]]@("", "plain", "two words", 'quote"inside', 'trail path\')
$environment = [ordered]@{
  Path = "child-path"
  YAP_NEW = "child-new"
  YAP_REMOVE = $null
}
$request = [Yap.NsisSmoke.LaunchRequest]::Create(
  $runtime,
  $arguments,
  $stdout,
  $stderr,
  $root,
  $environment
)

Assert-True ($request.ExecutablePath -ceq $runtime) "Executable path changed."
Assert-True ($request.Arguments.Count -eq 5) "Arguments were not retained as data."
Assert-True ($request.EnvironmentOverrides["Path"] -ceq "child-path") "Override was lost."
Assert-True ($request.EnvironmentRemovals -contains "YAP_REMOVE") "Removal was lost."
$arguments[1] = "caller-mutated"
$environment.Path = "caller-mutated"
Assert-True ($request.Arguments[1] -ceq "plain") "Caller mutation changed request arguments."
Assert-True ($request.EnvironmentOverrides["Path"] -ceq "child-path") "Caller mutation changed request environment."
Assert-True (
  [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildCommandLine($request) -ceq
  ('"' + $runtime + '" "" plain "two words" "quote\"inside" "trail path\\"')
) "Normal argument quoting changed."

$installDirectory = [Yap.NsisSmoke.NsisInstallDirectory]::Create("C:\Yap Test\Install")
$nsis = [Yap.NsisSmoke.LaunchRequest]::CreateNsisInstaller(
  $runtime,
  [string[]]@("/S"),
  $installDirectory,
  $stdout,
  $stderr,
  $null,
  [ordered]@{}
)
Assert-True (
  [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildCommandLine($nsis) -ceq
  ('"' + $runtime + '" /S /D=C:\Yap Test\Install')
) "NSIS /D= was not the literal final tail."

$entries = [string[]]@(
  '=C:=C:\work',
  'PATH=parent-path',
  'yap_remove=parent-remove',
  'ZED=last'
)
$block = [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildEnvironmentBlockText($request, $entries)
Assert-True ($block.EndsWith("`0`0")) "Environment block lacks its double-NUL terminator."
Assert-True ($block.Contains("=C:=C:\work`0")) "Hidden drive entry was lost."
Assert-True ($block.Contains("Path=child-path`0")) "Case-insensitive override was not applied."
Assert-True ($block.IndexOf("YAP_REMOVE=", [StringComparison]::OrdinalIgnoreCase) -lt 0) "Case-insensitive removal failed."
Assert-True ($block.Contains("YAP_NEW=child-new`0")) "New environment entry was lost."

$emptyEnvironmentRequest = [Yap.NsisSmoke.LaunchRequest]::Create(
  $runtime, @(),
  (Join-Path $root "empty.stdout.log"),
  (Join-Path $root "empty.stderr.log"),
  $null, [ordered]@{}
)
$emptyBlock = [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildEnvironmentBlockText(
  $emptyEnvironmentRequest,
  [string[]]@()
)
Assert-True ($emptyBlock -ceq "`0`0") "An empty environment was not double-NUL terminated."

foreach ($operation in @(
  { [Yap.NsisSmoke.LaunchRequest]::Create("pwsh.exe", @(), $stdout, $stderr, $null, @{}) },
  { [Yap.NsisSmoke.LaunchRequest]::Create($runtime, @("bad`0arg"), $stdout, $stderr, $null, @{}) },
  { [Yap.NsisSmoke.LaunchRequest]::Create($runtime, @(), $stdout, $stdout, $null, @{}) },
  { [Yap.NsisSmoke.LaunchRequest]::Create($runtime, @(), "relative.log", $stderr, $null, @{}) },
  { [Yap.NsisSmoke.NsisInstallDirectory]::Create('C:\bad"path') },
  { [Yap.NsisSmoke.NsisInstallDirectory]::Create("C:\bad`npath") },
  { [Yap.NsisSmoke.LaunchRequest]::Create($runtime, @(), $stdout, $stderr, $null, [ordered]@{ '=C:' = 'mutate' }) },
  { [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildEnvironmentBlockText($request, [string[]]@('Path=one', 'PATH=two')) }
)) {
  $threw = $false
  try { & $operation } catch { $threw = $true }
  Assert-True $threw "Invalid launch input was accepted."
}

Write-Output "Windows contained-process request contracts passed."
```

Wrap the test body in `try/finally`. The `finally` block may remove `$root` only after re-reading the exact `.yap-launch-request-v1` sentinel and proving `$root` is the strict expected child of the system temp directory; a failed ownership check retains the directory for diagnosis.

- [ ] **Step 2: Run the contract and confirm RED**

Run from `desktop`:

```powershell
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/windows-contained-process.contract.test.ps1
```

Expected: non-zero exit because `windows-contained-process.cs` and its types do not exist.

- [ ] **Step 3: Implement request validation, one argument encoder, and the typed NSIS tail**

In `windows-contained-process.cs`, define sealed `NsisInstallDirectory` and `LaunchRequest` types in namespace `Yap.NsisSmoke`. Use private constructors and exactly two public factories. Validation must reject non-absolute or missing executables, non-absolute/equal stdout and stderr paths, non-absolute/missing working directories, NUL in any argument/name/value, invalid environment names, attempts to override names beginning with `=`, quotes/CR/LF/NUL in the NSIS directory, an NSIS factory argument list that already contains `/D=`, and command lines whose mutable buffer plus terminator exceeds 32,767 UTF-16 characters.

Use this command-line algorithm:

```csharp
internal string BuildCommandLine()
{
    StringBuilder value = new StringBuilder();
    value.Append('"').Append(ExecutablePath).Append('"');
    foreach (string argument in Arguments)
        value.Append(' ').Append(QuoteNormalArgument(argument));
    if (NsisDirectory != null)
        value.Append(" /D=").Append(NsisDirectory.Value);
    if (value.Length + 1 > 32767)
        throw new ArgumentException("Windows command line exceeds 32,767 UTF-16 characters.");
    return value.ToString();
}

internal static string QuoteNormalArgument(string argument)
{
    if (argument.IndexOf('\0') >= 0)
        throw new ArgumentException("Arguments must not contain NUL.", nameof(argument));
    bool quote = argument.Length == 0 || argument.Any(c => char.IsWhiteSpace(c) || c == '"');
    if (!quote)
        return argument;
    StringBuilder result = new StringBuilder("\"");
    int backslashes = 0;
    foreach (char current in argument)
    {
        if (current == '\\')
        {
            backslashes++;
            continue;
        }
        if (current == '"')
        {
            result.Append('\\', checked(backslashes * 2 + 1));
            result.Append('"');
            backslashes = 0;
            continue;
        }
        result.Append('\\', backslashes);
        result.Append(current);
        backslashes = 0;
    }
    result.Append('\\', checked(backslashes * 2));
    return result.Append('"').ToString();
}
```

Defensively copy the argument array and expose it through a read-only collection. Represent a null `IDictionary` value as removal and a non-null value as override. Copy both environment collections into case-insensitive read-only collections so caller mutation after construction cannot change the request. Attempts to mutate any returned collection must fail.

- [ ] **Step 4: Implement deterministic environment entry merging**

Add an internal `EnvironmentBlockBuilder.BuildEntries` that parses hidden entries at the second `=`, preserves them unchanged, rejects inherited names that differ only by case, applies removals and overrides case-insensitively, and uses the `OrdinalIgnoreCase` plus `Ordinal` tie-break only while ordering a copied list for final emission:

```csharp
internal static string BuildBlockText(LaunchRequest request, IEnumerable<string> inherited)
{
    Dictionary<string, string> values = new Dictionary<string, string>(
        StringComparer.OrdinalIgnoreCase);
    foreach (string entry in inherited)
    {
        int separator = entry.StartsWith("=", StringComparison.Ordinal)
            ? entry.IndexOf('=', 1)
            : entry.IndexOf('=');
        if (separator <= 0)
            throw new InvalidOperationException("Inherited environment entry is malformed.");
        string name = entry.Substring(0, separator);
        if (!values.TryAdd(name, entry.Substring(separator + 1)))
            throw new InvalidOperationException("Inherited environment contains duplicate names.");
    }
    foreach (string name in request.EnvironmentRemovals)
        values.Remove(name);
    foreach (KeyValuePair<string, string> item in request.EnvironmentOverrides)
    {
        values.Remove(item.Key);
        values.Add(item.Key, item.Value);
    }
    StringBuilder block = new StringBuilder();
    foreach (KeyValuePair<string, string> item in values
        .OrderBy(item => item.Key, StringComparer.OrdinalIgnoreCase)
        .ThenBy(item => item.Key, StringComparer.Ordinal))
        block.Append(item.Key).Append('=').Append(item.Value).Append('\0');
    if (block.Length == 0)
        block.Append('\0');
    return block.Append('\0').ToString();
}
```

The production native adapter will later source `inherited` from `GetEnvironmentStringsW`; this task keeps the pure merge separately testable.

- [ ] **Step 5: Add only the test-load probe**

In `windows-contained-process.testing.cs`, add namespace `Yap.NsisSmoke.Testing` and a public static `LaunchRequestProbe` whose two methods delegate only to the internal command-line and environment builders. Do not add a production environment switch or callback.

```csharp
public static class LaunchRequestProbe
{
    public static string BuildCommandLine(LaunchRequest request) => request.BuildCommandLine();

    public static string BuildEnvironmentBlockText(LaunchRequest request, string[] inherited) =>
        EnvironmentBlockBuilder.BuildBlockText(request, inherited);
}
```

- [ ] **Step 6: Load production C# from the helper module without loading tests**

At the top of `nsis-smoke-helpers.psm1`, compile only `windows-contained-process.cs` when `Yap.NsisSmoke.LaunchRequest` is absent:

```powershell
$containedProcessSource = Join-Path $PSScriptRoot "windows-contained-process.cs"
if (-not ("Yap.NsisSmoke.LaunchRequest" -as [type])) {
  Add-Type -Path $containedProcessSource
}
```

Leave the old embedded `KillOnCloseJob` temporarily in place for unmigrated consumers. Do not load `windows-contained-process.testing.cs` from production code.

- [ ] **Step 7: Run GREEN and commit**

Run the focused contract command from Step 2. Expected: exit 0 with `Windows contained-process request contracts passed.`

```powershell
git add desktop/tests/scripts/windows-contained-process.cs `
  desktop/tests/scripts/windows-contained-process.testing.cs `
  desktop/tests/scripts/windows-contained-process.contract.test.ps1 `
  desktop/tests/scripts/nsis-smoke-helpers.psm1
git commit -m "test: define contained process launch contracts"
```

---

### Task 2: Implement Retained-Handle Launcher And Lease Ownership

**Files:**
- Modify: `desktop/tests/scripts/windows-contained-process.cs`
- Modify: `desktop/tests/scripts/windows-contained-process.testing.cs`
- Modify: `desktop/tests/scripts/windows-contained-process.contract.test.ps1`

**Interfaces:**
- Produces: `WindowsContainedProcessLauncher.Launch(LaunchRequest) -> ContainedProcessLease`.
- Produces: `ContainedProcessLease.RootProcessId`, `RootCreationFileTime`, and `RootExecutablePath` immutable evidence.
- Produces: `RootExitReport WaitForRootExit(TimeSpan timeout)` with `Exited`, nullable unsigned `ExitCode`, and `ElapsedMilliseconds`.
- Produces: `JobQuiescenceReport WaitForQuiescence(TimeSpan timeout)`.
- Produces: `TerminationReport TerminateAndWait(uint exitCode, TimeSpan timeout)`.
- Produces: `ContainedProcessException.Stage`, `NativeErrorCode`, `CleanupProven`, and immutable `CleanupErrors`.
- Consumes: validated `LaunchRequest` from Task 1.

- [ ] **Step 1: Add failing lease state-machine contracts**

Extend `windows-contained-process.contract.test.ps1` to compile the test adapter and verify the state machine through a scripted OS adapter that never calls the real `CreateProcessW`. Add a finite, reference-cycle-safe `Get-ContainedProcessTestFailure` helper that walks `InnerException` and returns the first `Yap.NsisSmoke.ContainedProcessException`; PowerShell method invocation may wrap the typed exception, so tests must not assume it is always the outer exception or parse its message.

```powershell
$failingRequest = [Yap.NsisSmoke.LaunchRequest]::Create(
  $runtime,
  [string[]]@("-NoProfile", "-NonInteractive", "-Command", "exit 0"),
  (Join-Path $root "assign.stdout.log"),
  (Join-Path $root "assign.stderr.log"),
  $root,
  [ordered]@{}
)

foreach ($case in @(
  @{ Point = "OpenStdin"; Stage = "Redirect" },
  @{ Point = "OpenStdout"; Stage = "Redirect" },
  @{ Point = "OpenStderr"; Stage = "Redirect" },
  @{ Point = "CreateJob"; Stage = "CreateJob" },
  @{ Point = "ConfigureJob"; Stage = "CreateJob" },
  @{ Point = "InitializeAttributeList"; Stage = "CreateProcess" },
  @{ Point = "UpdateAttributeList"; Stage = "CreateProcess" },
  @{ Point = "CaptureEnvironment"; Stage = "CreateProcess" },
  @{ Point = "CreateProcess"; Stage = "CreateProcess" },
  @{ Point = "AssignJob"; Stage = "AssignJob" },
  @{ Point = "CaptureIdentity"; Stage = "CaptureIdentity" },
  @{ Point = "ResumeThread"; Stage = "Resume" }
)) {
  $scenario = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
  $scenario.FailurePoint = [Enum]::Parse(
    [Yap.NsisSmoke.Testing.ScriptedFailurePoint],
    [string]$case.Point
  )
  $candidate = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($scenario)
  $error = $null
  try { $candidate.Launch($failingRequest) } catch { $error = Get-ContainedProcessTestFailure $_.Exception }
  Assert-True ($error -is [Yap.NsisSmoke.ContainedProcessException]) "Scripted failure was untyped."
  Assert-True ($error.Stage.ToString() -ceq $case.Stage) "Scripted failure reported the wrong stage."
  Assert-True $error.CleanupProven "Scripted failure did not prove cleanup."
  Assert-True ($scenario.OpenHandleCount -eq 0) "Scripted failure leaked a test handle."
}

$cleanupFailure = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
$cleanupFailure.FailurePoint = [Yap.NsisSmoke.Testing.ScriptedFailurePoint]::ResumeThread
$cleanupFailure.CleanupWaitSignals = $false
$error = $null
try {
  [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($cleanupFailure).Launch($failingRequest)
} catch { $error = Get-ContainedProcessTestFailure $_.Exception }
Assert-True (-not $error.CleanupProven) "Failed cleanup was promoted to success."
Assert-True ($error.CleanupErrors.Count -gt 0) "Failed cleanup lost its evidence."
$cleanupErrorCount = $error.CleanupErrors.Count
$mutationThrew = $false
try { $error.CleanupErrors.Add("caller mutation") } catch { $mutationThrew = $true }
Assert-True $mutationThrew "CleanupErrors was mutable."
Assert-True ($error.CleanupErrors.Count -eq $cleanupErrorCount) "CleanupErrors changed after construction."

$releaseFailure = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
$releaseFailure.FailurePoint = [Yap.NsisSmoke.Testing.ScriptedFailurePoint]::ReleaseParentStdout
$error = $null
try {
  [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($releaseFailure).Launch($failingRequest)
} catch { $error = Get-ContainedProcessTestFailure $_.Exception }
Assert-True ($error.Stage.ToString() -ceq "Dispose") "Launch-only release reported the wrong stage."
Assert-True (-not $error.CleanupProven) "A launch-only close failure was promoted to proven cleanup."
Assert-True ($error.CleanupErrors.Count -gt 0) "Launch-only close failure lost cleanup evidence."
Assert-True ($releaseFailure.LeaseConstructionCount -eq 0) "A lease escaped after launch-only release failed."

foreach ($resumeCase in @(
  @{ Result = [uint32]0; NativeError = $null },
  @{ Result = [uint32]2; NativeError = $null },
  @{ Result = [uint32]::MaxValue; NativeError = 5 }
)) {
  $resume = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
  $resume.ResumeThreadResult = [uint32]$resumeCase.Result
  $resume.ResumeThreadLastError = 5
  $error = $null
  try {
    [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($resume).Launch($failingRequest)
  } catch { $error = Get-ContainedProcessTestFailure $_.Exception }
  Assert-True ($error.Stage.ToString() -ceq "Resume") "Unexpected suspend count reported the wrong stage."
  Assert-True ($error.NativeErrorCode -eq $resumeCase.NativeError) "Resume failure retained an inapplicable/stale native error."
  Assert-True $error.CleanupProven "Resume failure cleanup was not proven."
  Assert-True ($resume.ResumeThreadCallCount -eq 1) "ResumeThread was not called exactly once."
}

Launch at least eight scripted `uint.MaxValue` resume failures concurrently with distinct nonzero `ResumeThreadLastError` values. Collect the typed errors by scenario and assert each retains only its own code. This test must fail if the adapter exposes a mutable/shared `LastError` property.

Add a scripted identity-mismatch case whose captured image path differs from the canonical requested executable. Require stage `CaptureIdentity`, zero resume calls, proven bounded cleanup, and no returned lease.

Add table-driven lease-operation failures for `WaitForSingleObject`, `GetExitCodeProcess`, `QueryInformationJobObject`, and `TerminateJobObject`, each with a distinct captured code. Require the first three to report stage `Wait`, termination to report stage `Terminate`, every error to retain only its own per-call code, and no failed operation to manufacture cleanup proof.

$highExit = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
$highExit.RootInitiallyExited = $true
$highExit.RootExitCode = [uint32]0xF0000001
$highLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($highExit).Launch($failingRequest)
$highReport = $highLease.WaitForRootExit([TimeSpan]::FromSeconds(1))
Assert-True ($highReport.ExitCode -eq [uint32]0xF0000001) "A high DWORD exit code was narrowed."
$highLease.Dispose()

$recycled = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
$recycled.RootInitiallyExited = $true
$recycledLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($recycled).Launch($failingRequest)
$recycled.SimulateUnrelatedProcessWithSamePid()
$recycledLease.WaitForRootExit([TimeSpan]::FromSeconds(1)) | Out-Null
Assert-True ($recycled.ProcessReacquisitionCount -eq 0) "The lease reacquired ownership from a recycled PID."
$recycledLease.Dispose()

$success = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
$lease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($success).Launch($failingRequest)
$requiredOrder = @("CreateProcessSuspended", "AssignJob", "VerifyJobMembership", "CaptureIdentity", "ResumeThread")
$observedOrder = @($success.OperationLog | Where-Object { $_ -in $requiredOrder })
Assert-True (($observedOrder -join "|") -ceq ($requiredOrder -join "|")) "Success-path assignment/identity/resume order changed."
Assert-True ($success.ResumeThreadCallCount -eq 1) "Success path did not resume exactly once."
$first = $lease.TerminateAndWait(0x59504150, [TimeSpan]::FromSeconds(1))
$second = $lease.TerminateAndWait(0x59504150, [TimeSpan]::FromSeconds(1))
Assert-True ([object]::ReferenceEquals($first, $second)) "Idempotent termination did not return its original proof."
Assert-True ($success.JobTerminationCount -eq 1) "Idempotent termination signaled the Job twice."
$lease.Dispose()
$lease.Dispose()
Assert-True ($success.OpenHandleCount -eq 0) "Idempotent disposal leaked a test handle."
$disposedThrew = $false
try { $lease.WaitForRootExit([TimeSpan]::FromSeconds(1)) } catch [ObjectDisposedException] { $disposedThrew = $true }
Assert-True $disposedThrew "A disposed lease remained operable."
```

- [ ] **Step 2: Run the contract and confirm RED**

Run the Task 1 contract command. Expected: non-zero exit because launcher, lease, reports, and test factory are undefined.

- [ ] **Step 3: Add safe-handle types and immutable public reports**

In `windows-contained-process.cs`, add:

```csharp
public enum ContainedProcessStage
{
    Redirect,
    CreateJob,
    CreateProcess,
    AssignJob,
    CaptureIdentity,
    Resume,
    Wait,
    Terminate,
    Dispose
}

public sealed class RootExitReport
{
    public bool Exited { get; }
    public uint? ExitCode { get; }
    public long ElapsedMilliseconds { get; }

    internal RootExitReport(bool exited, uint? exitCode, long elapsedMilliseconds)
    {
        Exited = exited;
        ExitCode = exitCode;
        ElapsedMilliseconds = elapsedMilliseconds;
    }
}

public sealed class JobQuiescenceReport
{
    public bool Quiescent => true;
    public int PollIterations { get; }
    public long ElapsedMilliseconds { get; }

    internal JobQuiescenceReport(int pollIterations, long elapsedMilliseconds)
    {
        PollIterations = pollIterations;
        ElapsedMilliseconds = elapsedMilliseconds;
    }
}

public sealed class TerminationReport
{
    public uint RequestedExitCode { get; }
    public RootExitReport RootExit { get; }
    public JobQuiescenceReport Quiescence { get; }

    internal TerminationReport(
        uint requestedExitCode,
        RootExitReport rootExit,
        JobQuiescenceReport quiescence)
    {
        RequestedExitCode = requestedExitCode;
        RootExit = rootExit;
        Quiescence = quiescence;
    }
}
```

Use sealed `SafeProcessHandle`, `SafeThreadHandle`, `SafeJobHandle`, and `SafeRedirectHandle` subclasses of `SafeHandleZeroOrMinusOneIsInvalid`. Their only release operation is `CloseHandle`; no public raw-handle property is exposed, and the redirect type does not collide with the framework's `SafeFileHandle`.

- [ ] **Step 4: Add a production-fixed internal OS adapter**

Define internal `IWindowsProcessApi` and sealed `NativeWindowsProcessApi`. The public launcher constructor must always use `NativeWindowsProcessApi.Instance`; only an internal constructor accepts an adapter.

The native adapter must wrap these exact operations and capture `Marshal.GetLastWin32Error()` immediately on the calling thread: `CreateFileW`, `CreateJobObjectW`, `SetInformationJobObject`, `InitializeProcThreadAttributeList`, `UpdateProcThreadAttribute`, `CreateProcessW`, `AssignProcessToJobObject`, `IsProcessInJob`, `GetProcessId`, `GetProcessTimes`, `QueryFullProcessImageNameW`, `ResumeThread`, `WaitForSingleObject`, `GetExitCodeProcess`, `TerminateProcess`, `TerminateJobObject`, `QueryInformationJobObject`, `GetEnvironmentStringsW`, `FreeEnvironmentStringsW`, `DeleteProcThreadAttributeList`, and `CloseHandle`. Job creation sets only `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; it enables neither silent breakaway nor breakaway.

Every fallible wrapper returns one immutable per-call result containing its value/success state and applicable captured error code. `NativeWindowsProcessApi.Instance` has no mutable `LastError` or other per-call state and is safe for parallel launchers. Successful calls carry no error code even if the thread previously observed an error. Process/thread/job handles and parent-owned standard handles must be non-inheritable except the three child stdio handles explicitly created with inheritable security attributes.

- [ ] **Step 5: Implement the atomic launch transition**

Implement `WindowsContainedProcessLauncher.Launch` in this order:

```csharp
public ContainedProcessLease Launch(LaunchRequest request)
{
    if (request == null)
        throw new ArgumentNullException(nameof(request));
    LaunchResources resources = new LaunchResources();
    ContainedProcessStage stage = ContainedProcessStage.Redirect;
    try
    {
        resources.OpenStandardHandles(api, request);
        stage = ContainedProcessStage.CreateJob;
        resources.CreateKillOnCloseJob(api);
        resources.BuildAttributeList(api);
        resources.PinCommandLine(request.BuildCommandLine());
        resources.PinUnicodeEnvironment(api, request);
        stage = ContainedProcessStage.CreateProcess;
        resources.CreateSuspendedProcess(api, request);
        stage = ContainedProcessStage.AssignJob;
        resources.AssignAndVerifyJob(api);
        stage = ContainedProcessStage.CaptureIdentity;
        ProcessIdentity identity = resources.CaptureIdentity(api, request.ExecutablePath);
        stage = ContainedProcessStage.Resume;
        NativeCallResult<uint> resume = api.ResumeThread(resources.ThreadHandle);
        if (!resume.Succeeded)
            throw NativeFailure(stage, "ResumeThread failed.", resume.ErrorCode);
        if (resume.Value != 1)
            throw LogicalFailure(stage, "ResumeThread returned an unexpected suspend count.", nativeErrorCode: null);
        resources.CloseThread();
        stage = ContainedProcessStage.Dispose;
        resources.ReleaseLaunchOnlyResources(api);
        return resources.TransferLease(identity, api);
    }
    catch (Exception error)
    {
        LaunchCleanupResult cleanup = resources.CleanupFailedLaunch(api, TimeSpan.FromSeconds(5));
        throw ContainedProcessException.From(stage, error, cleanup);
    }
    finally
    {
        resources.DisposeRemainingResourcesNoThrow();
    }
}
```

`CreateSuspendedProcess` must pass the absolute executable as non-null `lpApplicationName`; a mutable command line; null process/thread security attributes; `bInheritHandles=TRUE`; `CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT`; the pinned environment block; optional absolute working directory; and `STARTF_USESTDHANDLES` with valid stdin/stdout/stderr plus a three-handle `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.

`CaptureIdentity` requires a positive process ID, preserves the full creation `FILETIME`, grows the `QueryFullProcessImageNameW` buffer only within the Windows path limit, canonicalizes the observed image path, and compares it case-insensitively with the canonical requested executable before resume. Any mismatch is a `CaptureIdentity` failure and enters Job cleanup without calling `ResumeThread`.

`ReleaseLaunchOnlyResources` runs before `TransferLease`: it closes parent stdio copies, verifies every explicit close result, deletes/frees the attribute list, and frees the inherited-handle array, command buffer, and environment buffer. A failure here enters `CleanupFailedLaunch` while `LaunchResources` still owns the process and Job; the lease is not constructed and cannot be lost behind a throwing `finally`. The final no-throw disposer is only a backstop for resources still owned after the success/failure path. Transfer only the original process and Job handles after launch-only release succeeds.

`CleanupFailedLaunch` has one five-second monotonic budget. If no process was created, cleanup is proven only after every acquired resource closes. If a process exists but was not assigned, call `TerminateProcess` through its retained handle and wait for that handle to signal. If it was assigned, call `TerminateJobObject`, wait for the retained root handle, and query Job quiescence. In every case, release and verify every owned parent stdio handle, thread handle, attribute-list allocation, inherited-handle array, command buffer, and environment buffer. Accumulate cleanup errors without replacing the primary stage/error. Set `CleanupProven=true` only when every launch-only resource released successfully, the created root (if any) is signaled, and the Job is quiescent whenever assignment occurred or resume may have occurred. Any release/close failure keeps proof false even if the root and Job are clean; no partial lease returns.

- [ ] **Step 6: Implement the non-detachable lease**

`ContainedProcessLease` owns the two safe handles and exposes only evidence plus these operations:

```csharp
public RootExitReport WaitForRootExit(TimeSpan timeout)
{
    EnsureOpen();
    ValidatePositiveTimeout(timeout);
    Stopwatch timer = Stopwatch.StartNew();
    NativeCallResult<uint> waitCall = api.WaitForSingleObject(
        processHandle,
        ToBoundedMilliseconds(timeout));
    if (!waitCall.Succeeded)
        throw NativeFailure(ContainedProcessStage.Wait, "Process wait failed.", waitCall.ErrorCode);
    uint wait = waitCall.Value;
    if (wait == NativeConstants.WaitTimeout)
        return new RootExitReport(false, null, timer.ElapsedMilliseconds);
    if (wait != NativeConstants.WaitObject0)
        throw LogicalFailure(ContainedProcessStage.Wait, "Process wait returned an unexpected value.", null);
    NativeCallResult<uint> exitCode = api.GetExitCode(processHandle);
    if (!exitCode.Succeeded)
        throw NativeFailure(ContainedProcessStage.Wait, "Process exit-code query failed.", exitCode.ErrorCode);
    return new RootExitReport(true, exitCode.Value, timer.ElapsedMilliseconds);
}

public JobQuiescenceReport WaitForQuiescence(TimeSpan timeout)
{
    EnsureOpen();
    ValidatePositiveTimeout(timeout);
    Stopwatch timer = Stopwatch.StartNew();
    int iterations = 0;
    while (timer.Elapsed < timeout)
    {
        iterations++;
        NativeCallResult<uint> active = api.QueryActiveProcessCount(jobHandle);
        if (!active.Succeeded)
            throw NativeFailure(ContainedProcessStage.Wait, "Job state query failed.", active.ErrorCode);
        if (active.Value == 0)
            return new JobQuiescenceReport(iterations, timer.ElapsedMilliseconds);
        Thread.Sleep(Math.Min(50, RemainingMilliseconds(timeout, timer)));
    }
    throw new ContainedProcessException(
        ContainedProcessStage.Wait,
        "Job quiescence was not proven before the deadline.",
        null,
        false,
        Array.Empty<string>());
}

public TerminationReport TerminateAndWait(uint exitCode, TimeSpan timeout)
{
    EnsureOpen();
    ValidatePositiveTimeout(timeout);
    Stopwatch timer = Stopwatch.StartNew();
    NativeCallResult<bool> terminate = api.TerminateJob(jobHandle, exitCode);
    if (!terminate.Succeeded || !terminate.Value)
        throw NativeFailure(ContainedProcessStage.Terminate, "Job termination failed.", terminate.ErrorCode);
    RootExitReport root = WaitForRootExit(Remaining(timeout, timer));
    if (!root.Exited)
        throw CleanupNotProven("Root process did not signal after Job termination.");
    JobQuiescenceReport quiescence = WaitForQuiescence(Remaining(timeout, timer));
    return new TerminationReport(exitCode, root, quiescence);
}
```

`Dispose` must be idempotent and no-throw, close process first and Job second, and never claim quiescence. SafeHandle finalization remains the emergency backstop. Do not expose active counts, process lists, raw handles, a managed `Process`, `Detach`, or `ReleaseOwnership`.

Serialize public lease operations with a private state lock. Cache the first successful `TerminationReport`; repeated `TerminateAndWait` calls return that same proof without signaling by PID or re-terminating an unrelated process. Calls after disposal throw `ObjectDisposedException`, while repeated `Dispose` calls remain no-throw.

- [ ] **Step 7: Add the test-only fault adapter**

Extend `windows-contained-process.testing.cs` first with `ScriptedNativeScenario`, `ScriptedFailurePoint`, a no-real-process `ScriptedWindowsProcessApi`, and `ContainedProcessTestFactory.CreateScriptedLauncher`. Its handles are test-owned kernel event handles, its operation log is immutable to callers, and it exposes only counters needed to prove cleanup/idempotence. Support distinct per-call wait/exit-code/query/termination failures, root-wait timeout, Job-quiescence timeout, explicit `ResumeThread` return/last-error values, and distinct release failures for parent stdio, thread, attribute list, command buffer, and environment buffer; tests require accumulated immutable cleanup errors, false proof after any release failure, zero lease construction, and the original stage/error to remain primary. `SimulateUnrelatedProcessWithSamePid` changes only fake external state and must never cause a process-open/reacquisition call. The scripted adapter must never call `CreateProcessW`.

Also define `InjectedFailurePoint` and a decorator over `NativeWindowsProcessApi` for Task 4's real integration suite, exposed only as `CreateFaultingNativeLauncher`. Compile this file only when a focused contract/integration test explicitly includes it. `AssignJob`, `CaptureIdentity`, and `ResumeThread` injection occurs before the real operation that could let child code run; `CreateProcess` injection creates no process.

- [ ] **Step 8: Run GREEN and commit**

Run the Task 1 command. Expected: exit 0; every scripted acquisition failure reports its typed stage, successful cleanup closes every test handle, failed cleanup remains false with accumulated evidence, and termination/disposal are idempotent.

```powershell
git add desktop/tests/scripts/windows-contained-process.cs `
  desktop/tests/scripts/windows-contained-process.testing.cs `
  desktop/tests/scripts/windows-contained-process.contract.test.ps1
git commit -m "feat: own contained Windows process lifetime"
```

---

### Task 3: Add Atomic Nonce-Bound Native Evidence

**Files:**
- Create: `desktop/tests/scripts/contained-process-evidence.psm1`
- Create: `desktop/tests/scripts/contained-process-evidence-child.ps1`
- Create: `desktop/tests/scripts/contained-process-evidence.test.ps1`

**Interfaces:**
- Produces: `New-NativeEvidenceRun -ResultRoot -ExpectedExecutable` returning immutable `Nonce`, `RunRoot`, `TemporaryPath`, and `FinalPath`.
- Produces: `Wait-NativeEvidence -Lease -Run -TimeoutSeconds` returning schema version, nonce, PID, and canonical process path.
- Consumes: `ContainedProcessLease.RootProcessId` and `RootExecutablePath` from Task 2.
- Does not authorize wait, termination, cleanup, or filesystem deletion.

- [ ] **Step 1: Write adversarial RED tests**

Create `contained-process-evidence.test.ps1` and cover: valid complete publication; final path invisible before rename; empty/partial temporary content ignored; malformed final JSON terminal; stale nonce terminal; extra/missing schema keys terminal; zero/negative/overflow PID terminal; PID mismatch terminal; path mismatch terminal; UTF-8 BOM and surrounding whitespace accepted; destination preexistence terminal; paused writer before rename; and two parallel runs with no cross-talk.

Use a fake lease object only for parsing tests:

```powershell
$lease = [pscustomobject]@{
  RootProcessId = $PID
  RootExecutablePath = [IO.Path]::GetFullPath([Environment]::ProcessPath)
}
$run = New-NativeEvidenceRun -ResultRoot $resultRoot -ExpectedExecutable $lease.RootExecutablePath
$record = [ordered]@{
  schemaVersion = 1
  nonce = $run.Nonce
  processId = $lease.RootProcessId
  processPath = $lease.RootExecutablePath
} | ConvertTo-Json -Compress
[IO.File]::WriteAllText($run.FinalPath, $record, [Text.UTF8Encoding]::new($false))
$observed = Wait-NativeEvidence -Lease $lease -Run $run -TimeoutSeconds 1
Assert-True ($observed.processId -eq $PID) "Valid evidence did not round-trip."
```

- [ ] **Step 2: Run the evidence test and confirm RED**

```powershell
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/contained-process-evidence.test.ps1
```

Expected: non-zero exit because the module and child fixture do not exist.

- [ ] **Step 3: Implement run creation and strict parent validation**

In `contained-process-evidence.psm1`, create a 16-byte nonce with `RandomNumberGenerator.Fill`, lowercase hex it, reject an existing `evidence-<nonce>` path, and create that directory with `New-Item -ItemType Directory -ErrorAction Stop` and no `Force`. Immediately create `.yap-native-evidence-v1` inside it with `FileMode.CreateNew`; any preexistence or creation collision is terminal. Define final name `identity-v1.json` and sibling temporary name `.identity-v1.<nonce>.tmp`.

`Wait-NativeEvidence` must use `Stopwatch`, bounded 5/10/20/40/50ms polling, and no retry after the final path first appears. Read once, parse with `ConvertFrom-Json -AsHashtable`, require exactly `schemaVersion`, `nonce`, `processId`, and `processPath`, require version 1, exact nonce, integral PID from 1 through `Int32.MaxValue`, exact lease PID, and case-insensitive canonical path equality among record, lease, and expected executable. A malformed or mismatched final record throws immediately.

- [ ] **Step 4: Implement the child publisher**

Create `contained-process-evidence-child.ps1` with mandatory `Nonce`, `TemporaryPath`, and `FinalPath`, plus test-only `PauseBeforeRenameMilliseconds` bounded from 0 through 5000. Require `Nonce` to match exactly 32 lowercase hexadecimal characters, the final leaf to be `identity-v1.json`, and the temporary leaf to be `.identity-v1.$Nonce.tmp`. It must reject different parent directories, reparse-point parents, and a preexisting destination, then publish exactly once:

```powershell
$record = [ordered]@{
  schemaVersion = 1
  nonce = $Nonce
  processId = $PID
  processPath = [IO.Path]::GetFullPath([Environment]::ProcessPath)
}
$json = $record | ConvertTo-Json -Compress
$stream = [IO.FileStream]::new(
  $TemporaryPath,
  [IO.FileMode]::CreateNew,
  [IO.FileAccess]::Write,
  [IO.FileShare]::None,
  4096,
  [IO.FileOptions]::WriteThrough
)
try {
  $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false), 4096, $true)
  try { $writer.Write($json); $writer.Flush(); $stream.Flush($true) } finally { $writer.Dispose() }
} finally {
  $stream.Dispose()
}
if ($PauseBeforeRenameMilliseconds -gt 0) {
  Start-Sleep -Milliseconds $PauseBeforeRenameMilliseconds
}
[IO.File]::Move($TemporaryPath, $FinalPath, $false)
```

There is no overwrite, copy, cross-volume fallback, or lifecycle action.

- [ ] **Step 5: Run GREEN and commit**

Run the Step 2 command. Expected: exit 0 and deterministic completion of every adversarial case without sleeps used as readiness proof.

```powershell
git add desktop/tests/scripts/contained-process-evidence.psm1 `
  desktop/tests/scripts/contained-process-evidence-child.ps1 `
  desktop/tests/scripts/contained-process-evidence.test.ps1
git commit -m "test: publish atomic contained process evidence"
```

---

### Task 4: Prove The Real Windows Boundary Under Adversarial Lifecycles

**Files:**
- Create: `desktop/tests/scripts/windows-contained-process-fixture.ps1`
- Create: `desktop/tests/scripts/windows-contained-process.integration.test.ps1`
- Modify: `desktop/tests/scripts/windows-contained-process.testing.cs`

**Interfaces:**
- Consumes only the production `LaunchRequest`, `WindowsContainedProcessLauncher`, and `ContainedProcessLease` APIs from Tasks 1–2.
- Loads `windows-contained-process.testing.cs` only in this focused test process.
- Uses `contained-process-evidence.psm1` only to corroborate the retained root PID/path; evidence never selects a process to wait for or terminate.

- [ ] **Step 1: Write the real-process matrix before changing the smoke harness**

Create `windows-contained-process.integration.test.ps1` with `#requires -Version 7.4`, `#requires -PSEdition Core`, strict mode, a suite-wide 90-second watchdog, and per-case stdout/stderr paths. Its normal mode creates a cryptographic 128-bit nonce root by rejecting preexistence and calling `New-Item -ItemType Directory -ErrorAction Stop` without `Force`, then creates `.yap-contained-integration-v1` via `FileMode.CreateNew`; its cleanup verifies the sentinel and strict temp-root containment before deletion. Compile the production and testing C# files together once, then import the evidence module and the production helper module for `Assert-NoProcessesUnderPath` only. The module sees the production C# type already loaded and must not compile a second copy.

Add a mandatory-argument `-NestedChild` mode to the same integration script. That mode runs only one inner launcher/descendant/quiescence case, atomically publishes a nonce-bound `nested-result-v1.json`, and exits; it never recursively runs the full matrix. The normal test launches this mode through an outer lease, so the child shell is provably in the outer Job before it creates the inner Job.

Create `windows-contained-process-fixture.ps1` for descendant readiness. It launches a case-local copy of `%SystemRoot%\System32\ping.exe` through one retained managed `Process` object, writes a strict nonce/PID/canonical-path record to a sibling temporary file, flushes/closes, renames without overwrite to `descendant-v1.json`, and then waits on that same managed object. The parent treats this record only as readiness evidence; Job termination and quiescence remain authoritative.

Implement these cases without retrying a failed case:

1. A child exits naturally with code 23; `WaitForRootExit` reports `Exited=true`, `ExitCode=23`, and the retained creation FILETIME/path remain stable after exit.
2. A child exits immediately; the lease still waits through the retained handle and never reacquires a managed `Process` by PID.
3. A long-running child publishes valid atomic evidence; `WaitForRootExit(250ms)` reports `Exited=false`, then `TerminateAndWait(0x59504150, 5s)` proves root exit and Job quiescence.
4. The fixture starts its case-local `ping.exe` descendant and atomically publishes the strict descendant record; `TerminateAndWait` ends both members before the report says quiescent and the executable-path residual audit is empty.
5. An outer lease launches `windows-contained-process.integration.test.ps1 -NestedChild`; that child creates and terminates an inner lease/descendant, atomically reports success, and exits, proving nested-Job behavior on Windows 10+ without assigning the current test process irreversibly.
6. Distinct stdout/stderr files receive their matching streams and can be opened with `FileShare.None` immediately after lease disposal.
7. The child sees case-insensitive environment overrides, removals, a new value, and the unchanged current-directory drive entry; the parent environment remains unchanged.
8. The child observes the exact requested working directory.
9. `CreateFaultingNativeLauncher` injects real-native `CreateProcess`, `AssignJob`, `CaptureIdentity`, and pre-operation `ResumeThread` failures; each reports the matching stage, never creates the post-resume marker, and finishes with `CleanupProven=true`.
10. Eight leases run in parallel with unique evidence nonces and result roots; no lease consumes another run's evidence or logs.
11. After an immediate root exit but before lease disposal, create and reap at least 512 unrelated short-lived processes; repeated retained-handle waits return the same root exit/identity without any process reacquisition by PID.
12. After every case, retained-handle root exit and Job quiescence are proven, all leases are disposed, and all test-owned files are exclusively openable. Long-running/descendant cases use a case-local copy of the signed Windows inbox `ping.exe`; after cleanup the bounded executable-path audit must find zero processes under that case root. Shared-PowerShell evidence cases rely on the lease proof rather than pretending a system-wide `pwsh.exe` path audit can distinguish the child.

The test command used for children must be data, not a raw command-line tail:

```powershell
$request = [Yap.NsisSmoke.LaunchRequest]::Create(
  [IO.Path]::GetFullPath([Environment]::ProcessPath),
  [string[]]@(
    "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
    "-File", (Join-Path $PSScriptRoot "contained-process-evidence-child.ps1"),
    "-Nonce", $run.Nonce,
    "-TemporaryPath", $run.TemporaryPath,
    "-FinalPath", $run.FinalPath
  ),
  $stdout,
  $stderr,
  $caseRoot,
  [ordered]@{}
)
$lease = [Yap.NsisSmoke.WindowsContainedProcessLauncher]::new().Launch($request)
$observed = Wait-NativeEvidence -Lease $lease -Run $run -TimeoutSeconds 5
Assert-True ($observed.processId -eq $lease.RootProcessId) "Evidence did not match the retained root."
```

- [ ] **Step 2: Run the integration suite and confirm RED**

From `desktop`:

```powershell
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/windows-contained-process.integration.test.ps1
```

Expected: non-zero exit on the first missing behavior or test adapter needed by the real-process matrix.

- [ ] **Step 3: Complete only the native behavior exposed by the matrix**

Adjust `windows-contained-process.testing.cs` only when a case needs a test adapter that cannot be expressed through production interfaces. Test adapters may expose injected native failures and immutable observation snapshots, but must not add test switches to production types, alter the process environment, or introduce a production fallback.

Every case must use bounded `Stopwatch` waits. `Start-Sleep` is allowed only inside a child fixture to keep that fixture alive or exercise the evidence pause point; it is never readiness or cleanup proof in the parent.

- [ ] **Step 4: Run GREEN, repeat once for race sensitivity, and commit**

Run the Step 2 command twice as two independent invocations. Expected: both exit 0; each invocation creates a fresh result root and completes inside 90 seconds.

```powershell
git add desktop/tests/scripts/windows-contained-process.testing.cs `
  desktop/tests/scripts/windows-contained-process-fixture.ps1 `
  desktop/tests/scripts/windows-contained-process.integration.test.ps1
git commit -m "test: verify contained Windows process lifecycles"
```

---

### Task 5: Add Thin PowerShell Adapters And Migrate Bounded Invocations

**Files:**
- Modify: `desktop/tests/scripts/nsis-smoke-helpers.psm1:1-12,1353-1592,1664-1686`
- Modify: `desktop/tests/scripts/nsis-smoke-helpers.test.ps1:187-633`
- Modify: `desktop/tests/scripts/smoke-nsis.ps1:93-150,277-287,401-413,445-486,518-624`

**Interfaces:**
- Produces: `New-YapContainedLaunchRequest -FilePath -ArgumentList -StdoutPath -StderrPath [-WorkingDirectory] [-Environment]`.
- Produces: `New-YapNsisInstallerLaunchRequest -FilePath -ArgumentList -InstallDirectory -StdoutPath -StderrPath [-WorkingDirectory] [-Environment]`.
- Produces: `Start-YapContainedProcess -Request`, returning exactly one `ContainedProcessLease`.
- Produces: `Invoke-YapContainedProcessWithDeadline -Request -TimeoutSeconds -CleanupTimeoutSeconds`, returning a serialization-safe invocation report only after root exit and Job quiescence are proven.
- Produces: `Invoke-YapAuthorizedCleanupMutation -CleanupAuthorized -Name -RetainedPaths -Action`, which never invokes `Action` when authorization is false and never changes authorization itself.
- Does not expose a raw Job, managed `Process`, process-tree list, PID termination function, raw NSIS tail, or fault-injection parameter.

- [ ] **Step 1: Add failing adapter and evidence-shape contracts**

In `nsis-smoke-helpers.test.ps1`, replace the live PID/CIM/Job cases with contract cases that import the module and assert:

```powershell
$request = New-YapContainedLaunchRequest `
  -FilePath ([Environment]::ProcessPath) `
  -ArgumentList @("-NoProfile", "-Command", "exit 7") `
  -StdoutPath (Join-Path $testRoot "bounded.stdout.log") `
  -StderrPath (Join-Path $testRoot "bounded.stderr.log")
$report = Invoke-YapContainedProcessWithDeadline `
  -Request $request `
  -TimeoutSeconds 5 `
  -CleanupTimeoutSeconds 5
Assert-True (-not $report.timedOut) "Natural exit was reported as a timeout."
Assert-True ($report.rootExit.exited -and $report.rootExit.exitCode -eq 7) "Root exit was not retained."
Assert-True $report.jobQuiescence.quiescent "Job quiescence was not proven."
Assert-True ($report.rootProcessId -gt 0) "Root PID was not retained as evidence."
Assert-True ($report.rootCreationFileTime -gt 0) "Creation identity was not retained as evidence."
Assert-True ([IO.Path]::IsPathFullyQualified($report.rootExecutablePath)) "Root path was not canonical."
```

Add a timeout case which returns `timedOut=true` only after a successful `TerminationReport`. Require the report's exact top-level keys and the exact nested termination keys/types shown below; reject extra/missing fields and signed narrowing of the requested/observed exit codes. Assert the public command parameters contain none of `Job`, `Process`, `ProcessId`, `RawArguments`, `FailAssignmentForTest`, `SnapshotProviderScript`, or `EnvironmentProviderScript`.

Add a static contract over `smoke-nsis.ps1` that requires schema version 2 and the top-level `cleanupAuthority` object with exactly `authorized`, `failureStage`, and `retainedPaths`.

Add a behavioral fail-closed fixture using ordinary dummy files for install/data/registry/shortcut resources:

```powershell
$protected = @("install", "data", "registry", "shortcut") | ForEach-Object {
  $path = Join-Path $testRoot "$_.sentinel"
  [IO.File]::WriteAllText($path, $_)
  $path
}
$blocked = Invoke-YapAuthorizedCleanupMutation `
  -CleanupAuthorized:$false `
  -Name "blocked-fixture-cleanup" `
  -RetainedPaths $protected `
  -Action { $protected | Remove-Item -Force }
Assert-True (-not $blocked.executed) "Unauthorized mutation was reported as executed."
Assert-True (($protected | Where-Object { -not (Test-Path -LiteralPath $_) }).Count -eq 0) "Unauthorized cleanup touched a protected resource."
Assert-True (@($blocked.retainedPaths).Count -eq 4) "Blocked cleanup lost retained-resource evidence."
```

Use a separate disposable file with `CleanupAuthorized:$true` to prove the action runs exactly once. A thrown authorized action must propagate and must not manufacture cleanup proof.

- [ ] **Step 2: Run the affected helper contract and confirm RED**

```powershell
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/nsis-smoke-helpers.test.ps1
```

Expected: non-zero exit because the new adapter commands and evidence schema do not exist.

- [ ] **Step 3: Load the production boundary once and implement the five thin adapters**

At module import, resolve `windows-contained-process.cs` relative to `$PSScriptRoot`, load it exactly once with `Add-Type -Path`, and fail import if its SHA/path cannot be read or its expected production type is absent. Never load `windows-contained-process.testing.cs` from the module.

`Invoke-YapContainedProcessWithDeadline` must:

1. Launch exactly one lease.
2. Wait for root exit for the runtime deadline.
3. If the root exits, wait separately for Job quiescence.
4. If the runtime deadline expires, call `TerminateAndWait(0x59504150, cleanupTimeout)` and return `timedOut=true` only if both root exit and quiescence are proven.
5. Copy only scalar/report fields into a serialization-safe ordered object.
6. Dispose the lease in `finally`; disposal alone never changes a failed proof into success.
7. Throw on an unproven root exit or unproven Job quiescence. Preserve `ContainedProcessException.Stage`, `NativeErrorCode`, `CleanupProven`, and `CleanupErrors` as the inner typed failure.

The invocation report shape is fixed:

```powershell
[ordered]@{
  timedOut = [bool]
  rootProcessId = [int]
  rootCreationFileTime = [long]
  rootExecutablePath = [string]
  rootExit = [ordered]@{ exited = [bool]; exitCode = [Nullable[uint32]]; elapsedMilliseconds = [long] }
  jobQuiescence = [ordered]@{ quiescent = [bool]; elapsedMilliseconds = [long] }
  termination = $null
}
```

For natural completion `termination` is exactly null. For a proven timeout it is exactly:

```powershell
[ordered]@{
  requestedExitCode = [uint32]
  rootExit = [ordered]@{ exited = [bool]; exitCode = [Nullable[uint32]]; elapsedMilliseconds = [long] }
  jobQuiescence = [ordered]@{ quiescent = [bool]; elapsedMilliseconds = [long] }
}
```

- [ ] **Step 4: Introduce one fail-closed cleanup authorization gate**

In `smoke-nsis.ps1`, bump `schemaVersion` to 2 and initialize:

```powershell
$filesystemCleanupAuthorized = $true
$evidence.cleanupAuthority = [ordered]@{
  authorized = $true
  failureStage = $null
  retainedPaths = @()
}
```

Immediately before every owned process launch, set the gate and evidence field to `false`. Set them back to `true` only after the call returns explicit root-exit and Job-quiescence proof, or after a launch exception explicitly says `CleanupProven=true`. Any other exception leaves the gate false, records its typed stage when available, and records all installer-owned paths in `retainedPaths`.

Do not infer cleanup authority from process absence in CIM, a missing PID, a deleted path, a caught exception, or lease disposal.

Add a private `Get-ContainedProcessFailure` helper in `smoke-nsis.ps1` that walks the finite `InnerException` chain with reference-cycle detection and returns the first `Yap.NsisSmoke.ContainedProcessException`, or null. This handles PowerShell's method-invocation wrapper without parsing exception messages. Only that typed object's `CleanupProven=true` may restore the gate after a thrown launch.

Implement `Invoke-YapAuthorizedCleanupMutation` in the helper module as the sole gateway for install-root, smoke-root, quarantine, app-data, registry, and shortcut mutation. If unauthorized, it returns `{ executed=false; name; retainedPaths }` without invoking the action. If authorized, it invokes the action once and returns `{ executed=true; name; retainedPaths=@() }`. It neither catches action failures nor changes process-cleanup authorization.

- [ ] **Step 5: Migrate install, default uninstall, reinstall, explicit-data uninstall, and cleanup uninstall**

Use `New-YapNsisInstallerLaunchRequest` for install/reinstall so `/D=$installRoot` cannot be represented as a normal/raw tail. Use `New-YapContainedLaunchRequest` for every uninstaller. Store the fixed invocation reports under the existing `evidence.processes` phase keys; remove `ProcessIds` loops and the `trackedProcessIds` evidence field rather than preserving an empty vestige of tree ownership.

For a returned timeout report, first restore cleanup authorization because termination/quiescence was proven, then fail the smoke phase with a timeout error. For a non-zero natural exit, cleanup remains authorized but the smoke phase fails on the exit code.

In the `finally` cleanup-uninstall path, execute the uninstaller only while the gate is true. Mark the gate false immediately before it launches; restore it only from proof. If it becomes false, skip every later mutating cleanup operation.

- [ ] **Step 6: Keep only filesystem contracts in the legacy helper suite**

Delete the moved native process, PID identity, CIM tree, and Job fault-injection cases from `nsis-smoke-helpers.test.ps1`. Preserve and run all path-token, path-containment, reparse-point, hash, validated-tree, quarantine, sentinel, smoke-lock, and bounded residual-snapshot tests. The focused contract/integration suites now own native lifecycle behavior.

- [ ] **Step 7: Run focused GREEN checks and commit**

```powershell
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/nsis-smoke-helpers.test.ps1
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/windows-contained-process.contract.test.ps1
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/windows-contained-process.integration.test.ps1
```

Expected: all three exit 0. Do not run the NSIS bundle yet because the installed-app owner is intentionally migrated in Task 6.

```powershell
git add desktop/tests/scripts/nsis-smoke-helpers.psm1 `
  desktop/tests/scripts/nsis-smoke-helpers.test.ps1 `
  desktop/tests/scripts/smoke-nsis.ps1
git commit -m "refactor: contain NSIS process invocations"
```

---

### Task 6: Migrate The App Lease And Delete Split Lifecycle Ownership

**Files:**
- Modify: `desktop/tests/scripts/smoke-nsis.ps1:93-150,355-395,510-647`
- Modify: `desktop/tests/scripts/nsis-smoke-helpers.psm1:844-1290,1353-1661,1664-1686`
- Modify: `desktop/tests/scripts/nsis-smoke-helpers.test.ps1`

**Interfaces removed:**
- `Test-ProcessAlive`
- `Test-ProcessIdentityAlive`
- `Get-ProcessTreeIds`
- PID/start-time identity normalization and tracked-tree stop helpers
- `Start-ProcessWithEnvironment`
- `Start-JobContainedProcess`
- `Stop-JobContainedProcess`
- `Invoke-ProcessWithDeadline`
- `Assert-ProcessSurvives`
- `ConvertTo-SerializableIdentityMap`
- `Add-TrackedProcess`, `trackedProcessIds`, `treeProcessIds`, and the legacy `DiscoveredProcessIds`, `TerminatedProcessIds`, `ResidualProcessIds`, `ReusedProcessIds`, and `ProcessIdentityById` evidence

**Interfaces retained only for post-disposal diagnostics:**
- `Get-ProcessSnapshot`, simplified to `ProcessId` and canonical `ExecutablePath`.
- `Get-ProcessesUnderPath` and `Assert-NoProcessesUnderPath`.
- Their bounded provider hook remains test-only diagnostic injection and cannot authorize cleanup.

- [ ] **Step 1: Add failing ownership and fail-closed contracts**

Extend `nsis-smoke-helpers.test.ps1` with source/AST assertions that:

- `smoke-nsis.ps1` contains one `$appLease` owner and no `$appProcess`, `$appProcessId`, `$appStartTime`, or `$appProcessIdentity` owner.
- None of the removed function definitions or exports remain.
- No call to `Start-Process`, `Stop-Process`, `Get-Process -Id`, `.Kill(`, or `.WaitForExit(` remains in `smoke-nsis.ps1`.
- Every `Invoke-YapAuthorizedCleanupMutation` call passes exactly the variable `$filesystemCleanupAuthorized` to `-CleanupAuthorized`; literals, negation, aliases, or a second authorization variable fail the contract.
- Every call to `Remove-OwnedDeleteQuarantine`, `Remove-ValidatedTree`, mutating `Remove-Item`, or footprint cleanup appears only inside an `Action` passed to `Invoke-YapAuthorizedCleanupMutation`; cleanup uninstall is separately guarded by `$filesystemCleanupAuthorized` before launch.
- In the same control-flow block, every owned `Start-YapContainedProcess`/`Invoke-YapContainedProcessWithDeadline` call is preceded by assignments setting both `$filesystemCleanupAuthorized` and `$evidence.cleanupAuthority.authorized` to false. Assignments of true are permitted only inside a branch that tests explicit root-exit plus Job-quiescence report fields or the typed `ContainedProcessException.CleanupProven`; no catch, disposal, CIM result, or path absence can dominate a true assignment.
- `Assert-NoProcessesUnderPath` is called only after explicit lease cleanup/disposal and its result never sets `$filesystemCleanupAuthorized`.

The AST contract may validate command names, bound parameter expressions, assignments, and control-flow ancestry; do not snapshot implementation text or exact line numbers. Add an unproven-failure policy fixture that initializes the real variable false, presents a typed/scalar cleanup result with `CleanupProven=false`, attempts all four dummy install/data/registry/shortcut actions through production gateway calls, and proves zero actions ran, all resources remain, all paths were retained, and neither authorization field became true.

- [ ] **Step 2: Run the helper contract and confirm RED**

Run the Task 5 helper command. Expected: non-zero exit while the app still uses managed `Process`, PID/start time, CIM discovery, and tree termination.

- [ ] **Step 3: Replace app launch/probe/termination with one lease**

Build the app request with `New-YapContainedLaunchRequest`, including the existing environment overrides and distinct stdout/stderr paths. Before launch, set both `$filesystemCleanupAuthorized` and `$evidence.cleanupAuthority.authorized` to false. The normal completion path is exactly:

```powershell
$appLease = $null
$appCleanupProven = $false
$appExitedDuringProbe = $false
$appError = $null
$evidence.processes.app = [ordered]@{
  rootProcessId = $null
  rootCreationFileTime = $null
  rootExecutablePath = $null
  survivalProbeSeconds = $launchProbeSeconds
  rootExit = $null
  jobQuiescence = $null
  termination = $null
  failure = $null
}
try {
  $appLease = Start-YapContainedProcess -Request $appRequest
  $evidence.processes.app["rootProcessId"] = $appLease.RootProcessId
  $evidence.processes.app["rootCreationFileTime"] = $appLease.RootCreationFileTime
  $evidence.processes.app["rootExecutablePath"] = $appLease.RootExecutablePath
  $probe = $appLease.WaitForRootExit([TimeSpan]::FromSeconds($launchProbeSeconds))
  if ($probe.Exited) {
    $quiescence = $appLease.WaitForQuiescence([TimeSpan]::FromSeconds($cleanupTimeoutSeconds))
    $evidence.processes.app.rootExit = [ordered]@{
      exited = [bool]$probe.Exited
      exitCode = if ($null -eq $probe.ExitCode) { $null } else { [uint32]$probe.ExitCode }
      elapsedMilliseconds = [long]$probe.ElapsedMilliseconds
    }
    $evidence.processes.app.jobQuiescence = [ordered]@{
      quiescent = [bool]$quiescence.Quiescent
      elapsedMilliseconds = [long]$quiescence.ElapsedMilliseconds
    }
    $appCleanupProven = [bool]($probe.Exited -and $quiescence.Quiescent)
    $appExitedDuringProbe = $true
  }
  else {
    $termination = $appLease.TerminateAndWait(0x59504150, [TimeSpan]::FromSeconds($cleanupTimeoutSeconds))
    $evidence.processes.app.rootExit = [ordered]@{
      exited = [bool]$termination.RootExit.Exited
      exitCode = if ($null -eq $termination.RootExit.ExitCode) { $null } else { [uint32]$termination.RootExit.ExitCode }
      elapsedMilliseconds = [long]$termination.RootExit.ElapsedMilliseconds
    }
    $evidence.processes.app.jobQuiescence = [ordered]@{
      quiescent = [bool]$termination.Quiescence.Quiescent
      elapsedMilliseconds = [long]$termination.Quiescence.ElapsedMilliseconds
    }
    $evidence.processes.app.termination = [ordered]@{
      requestedExitCode = [uint32]$termination.RequestedExitCode
      rootExit = $evidence.processes.app.rootExit
      jobQuiescence = $evidence.processes.app.jobQuiescence
    }
    $appCleanupProven = [bool]($termination.RootExit.Exited -and $termination.Quiescence.Quiescent)
  }
}
catch {
  $appError = $_
  $typedFailure = Get-ContainedProcessFailure $_.Exception
  if ($null -ne $typedFailure) {
    $evidence.processes.app.failure = [ordered]@{
      stage = [string]$typedFailure.Stage
      nativeErrorCode = if ($null -eq $typedFailure.NativeErrorCode) { $null } else { [int]$typedFailure.NativeErrorCode }
      cleanupProven = [bool]$typedFailure.CleanupProven
      cleanupErrorCount = [int]$typedFailure.CleanupErrors.Count
    }
    $appCleanupProven = [bool]$typedFailure.CleanupProven
  }
  else {
    $evidence.processes.app.failure = [ordered]@{
      stage = "untyped"
      nativeErrorCode = $null
      cleanupProven = $false
      cleanupErrorCount = 0
    }
  }
}

# Deliberately retain a non-null lease for the outer smoke finally. That single
# cleanup owner proves bounded termination when needed, disposes once, and nulls it.

if ($appCleanupProven) {
  $filesystemCleanupAuthorized = $true
  $evidence.cleanupAuthority.authorized = $true
}
elseif ($null -eq $appError) {
  $evidence.cleanupAuthority.failureStage = "app-cleanup-proof"
  throw "Installed application cleanup was not proven."
}
if ($null -ne $appError) {
  if (-not $appCleanupProven) {
    $evidence.cleanupAuthority.failureStage = [string]$evidence.processes.app.failure.stage
  }
  throw $appError
}
if ($appExitedDuringProbe) {
  throw "Installed application exited during the survival probe."
}
```

The survival probe is the retained process-handle wait itself. Do not insert a sleep, enumerate descendants, reopen the process by PID, or ask CIM whether the app survived. The catch records only typed scalar diagnostics, never raw exception text, arguments, or environment values. It deliberately leaves a successfully constructed `$appLease` non-null: ownership transfers to the one outer smoke `finally`, even when serialization or another post-launch operation throws. Before rethrowing an unproven error, add every installer-owned resource to `cleanupAuthority.retainedPaths`; the primary error remains the one rethrown. No catch retries through the legacy launcher.

- [ ] **Step 4: Make `finally` preserve evidence and paths when cleanup is uncertain**

At the start of the outer smoke `finally`, initialize `$finalAppCleanupProven=[bool]$appCleanupProven`. When `$appLease` is non-null and proof is still false, call `TerminateAndWait` once and serialize the same scalar `rootExit`, `jobQuiescence`, and `termination` fields shown above. Set `$finalAppCleanupProven=true` only when the returned report proves both root exit and Job quiescence. If it throws, append the cleanup error without replacing the primary smoke error, unwrap the typed failure, record its stage/native/cleanup fields, and set the local proof only when that typed failure explicitly reports `CleanupProven=true`. Whether proof was already established or the final attempt succeeds or fails, an inner `finally` calls `Dispose()` exactly once and immediately sets `$appLease=$null`. Only after that inner `finally` may the code restore the authorization gate from `$finalAppCleanupProven`; the outer `finally` must never call into a disposed non-null lease. If no primary error existed and final cleanup fails, promote the accumulated cleanup failure only after evidence and retained paths are recorded.

Route every authorized mutation through the gateway, for example:

```powershell
$mutation = Invoke-YapAuthorizedCleanupMutation `
  -CleanupAuthorized:$filesystemCleanupAuthorized `
  -Name "remove-install-quarantine" `
  -RetainedPaths @($installRoot, [string]$footprintPaths.deleteQuarantine) `
  -Action { Remove-OwnedDeleteQuarantine }
if (-not $mutation.executed) {
  $evidence.cleanupAuthority.retainedPaths = @($evidence.cleanupAuthority.retainedPaths + $mutation.retainedPaths | Sort-Object -Unique)
}
```

Use the same gateway around install-root, smoke-root, quarantine, app-data, registry, and shortcut mutations. Cleanup uninstall is the one launch operation rather than a mutation action: guard it before launch with `$filesystemCleanupAuthorized`, set the gate false for its lease, and restore it only from explicit lease proof. If cleanup remains unproven:

- do not run cleanup uninstall;
- do not remove or rename the install root, smoke root, delete quarantine, app-data directories, registry entries, or shortcuts;
- record each retained absolute path in `cleanupAuthority.retainedPaths`;
- run `Assert-NoProcessesUnderPath` only as read-only diagnostic evidence and never use success to restore authorization;
- release the smoke-run lock;
- write final evidence and throw the aggregate failure.

If authorization is true, proceed through cleanup uninstall and each existing sentinel/path/reparse validation. Set authorization false around cleanup uninstall exactly as for every other owned launch. If that proof fails, stop before the next mutating cleanup step and retain the remaining paths.

- [ ] **Step 5: Delete the embedded legacy boundary and ownership helpers**

Remove the embedded `Yap.NsisSmoke.KillOnCloseJob`, all loose `{ Job, Process }` plumbing, PID/start-time ownership, tree enumeration/termination, and the associated exports. Keep the bounded CIM snapshot only in the path-based residual audit and simplify its returned record to PID/path. It must not accept a root PID, creation identity, or termination callback.

Use `rg` to prove zero production references:

```powershell
rg -n "KillOnCloseJob|Test-ProcessAlive|Test-ProcessIdentityAlive|Get-ProcessTreeIds|ConvertTo-ProcessCreationIdentity|Normalize-ProcessCreationIdentity|Stop-VerifiedProcessHandle|Update-TrackedProcessIds|Get-TrackedProcessDepth|ConvertTo-SerializableIdentityMap|Stop-TrackedProcessesBounded|Stop-ProcessTreeBounded|Start-ProcessWithEnvironment|Start-JobContainedProcess|Stop-JobContainedProcess|Invoke-ProcessWithDeadline|Assert-ProcessSurvives|Add-TrackedProcess|trackedProcessIds|treeProcessIds|DiscoveredProcessIds|TerminatedProcessIds|ResidualProcessIds|ReusedProcessIds|ProcessIdentityById|appProcess(Id|Identity|StartTime)?" `
  desktop/tests/scripts/nsis-smoke-helpers.psm1 `
  desktop/tests/scripts/smoke-nsis.ps1
```

Expected: no matches.

- [ ] **Step 6: Run the focused boundary checks and one local test-installer smoke**

```powershell
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/nsis-smoke-helpers.test.ps1
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/windows-contained-process.contract.test.ps1
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/contained-process-evidence.test.ps1
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/windows-contained-process.integration.test.ps1
pnpm build:nsis:test
pnpm test:nsis:local
```

Expected: all commands exit 0; smoke evidence has `schemaVersion=2`, every completed process phase has retained-handle proof fields, and `cleanupAuthority.authorized=true` at successful completion.

- [ ] **Step 7: Commit the completed migration**

```powershell
git add desktop/tests/scripts/nsis-smoke-helpers.psm1 `
  desktop/tests/scripts/nsis-smoke-helpers.test.ps1 `
  desktop/tests/scripts/smoke-nsis.ps1
git commit -m "refactor: make the NSIS smoke lease authoritative"
```

---

### Task 7: Create One Canonical Harness And Separate Policy From Integration

**Files:**
- Create: `desktop/tests/scripts/contained-process-redaction.psm1`
- Create: `desktop/tests/scripts/contained-process-redaction.test.ps1`
- Create: `desktop/tests/scripts/contained-process-harness-runner.psm1`
- Create: `desktop/tests/scripts/contained-process-harness-runner.test.ps1`
- Create: `desktop/tests/scripts/run-contained-process-harness.ps1`
- Modify: `desktop/tests/scripts/release-evidence.contract.mjs:1779-1793`
- Verify unchanged: `desktop/package.json:14-25`

**Canonical entrypoint:**

```powershell
$runtime = [IO.Path]::GetFullPath([Environment]::ProcessPath)
$resultRoot = [IO.Path]::GetFullPath("./tests/results/contained-process-harness/example-$([Guid]::NewGuid().ToString('N'))")
& $runtime -File ./tests/scripts/run-contained-process-harness.ps1 `
  -PowerShellExecutable $runtime `
  -RuntimeLabel "local-current" `
  -ResultRoot $resultRoot `
  -Suite All
```

`PowerShellExecutable`, `RuntimeLabel`, and `ResultRoot` are mandatory. `Suite` defaults to `All` and exists only for proportional local development; every CI/release caller uses `All`.

- [ ] **Step 1: Make the policy contract RED on the new ownership boundary**

In `release-evidence.contract.mjs`, delete the test that spawns the PowerShell helper integration suite. Add policy assertions that:

- `desktop/package.json` keeps `test:release-contract` exactly as the Node 24 check plus `node --test ./tests/scripts/release-evidence.contract.mjs`;
- the package contains no script that invokes `run-contained-process-harness.ps1` through ambient `pwsh.exe`/`powershell.exe` resolution;
- the canonical file exists and its public parameter block requires `PowerShellExecutable`, `RuntimeLabel`, and `ResultRoot` and restricts `Suite` to the four values above;
- the canonical file imports `contained-process-harness-runner.psm1`, while neither `nsis-smoke-helpers.psm1` nor `smoke-nsis.ps1` imports that runner-only module;
- workflow integration ownership will be checked by Task 8, not executed by this Node test.

Run:

```powershell
pnpm test:release-contract
```

Expected: non-zero exit because the canonical harness file does not exist.

- [ ] **Step 2: Build and test the artifact redactor first**

Create `contained-process-redaction.test.ps1` before the module. Feed it synthetic lines containing `C:\Users\Alice`, repository/result/runtime paths, a secret-shaped path leaf, a fake `Authorization: Bearer test_secret_value`, `password=fake-password`, a GitHub-shaped fake token, a URL credential, an encoded-command body, and a line longer than the artifact limit. Require exact known-path replacement, secret-pattern replacement, deterministic SHA-256 path identity, redaction of any non-allowlisted or secret-shaped leaf name, bounded UTF-8 output, and a visible truncation marker. Also require benign stage, PID, nonce, exit-code, and fixed relative test-name fields to survive.

Run under the exact current runtime and confirm RED because the module is absent:

```powershell
& ([Environment]::ProcessPath) -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File ./tests/scripts/contained-process-redaction.test.ps1
```

Implement `contained-process-redaction.psm1` with two exported pure functions:

- `ConvertTo-ContainedProcessPathIdentity -Path -KnownRoots -AllowedLeafNames`, returning only `fileName`, lowercase `canonicalPathSha256`, and `locationClass` (`repository`, `result-root`, `runner-temp`, `program-files`, `user-profile`, or `other`). `fileName` is preserved only when it exactly matches a caller-supplied fixed allowlist and survives the secret-pattern scan; otherwise it is `[redacted-leaf]`;
- `ConvertTo-RedactedContainedProcessText -Text -KnownPaths -MaximumUtf8Bytes`, replacing exact known paths longest-first and case-insensitively, then credential/token/authorization/encoded-command patterns, and finally truncating on a UTF-8 character boundary.

The module never writes files and never returns the unredacted input as a side channel. Run the command again; expected GREEN.

- [ ] **Step 3: Implement strict runtime and result-root validation**

Create `run-contained-process-harness.ps1` with Core 7.4 requirements and strict mode. It must:

1. canonicalize the supplied executable, require an existing absolute leaf named exactly `pwsh.exe` case-insensitively, and allowlist only that fixed leaf in runtime path identity;
2. require it to equal `[IO.Path]::GetFullPath([Environment]::ProcessPath)` case-insensitively, proving the caller launched the entrypoint with the promised runtime;
3. require `PSEdition=Core`, version `>=7.4`, and a `RuntimeLabel` matching `^[a-z0-9][a-z0-9.-]{0,63}$` before it can enter JSON or filenames;
4. require `ResultRoot` to be an absolute strict child of `desktop/tests/results/contained-process-harness`, reject any existing reparse-point ancestor beneath that fixed base, create it only if absent, and create `.yap-contained-process-results-v1` with `FileMode.CreateNew` before any suite runs;
5. create one cryptographic 128-bit nonce subdirectory per invocation by rejecting preexistence and calling `New-Item -ItemType Directory -ErrorAction Stop` without `Force`;
6. create `private` and `artifact` subdirectories beneath the nonce directory;
7. resolve the repository root and canonical existing `desktop` directory from `$PSScriptRoot`, never from the caller's current directory.

An existing result root or sentinel is terminal. CI therefore supplies a unique root per lane/run attempt.

- [ ] **Step 4: Parse every tracked script under the exact runtime**

Use `git -C $repoRoot ls-files -- "*.ps1" "*.psm1"`, require at least one result, resolve every path beneath the repository root, and call `System.Management.Automation.Language.Parser.ParseFile`. Any parser error fails the run before integration. Record only the number of parsed files and error messages with repository-relative paths.

- [ ] **Step 5: Run each selected suite under a contained per-suite deadline**

The fixed suite map is:

| Suite selector | Scripts, in order |
|---|---|
| `Contract` | `nsis-smoke-helpers.test.ps1` (60s), `contained-process-redaction.test.ps1` (30s), `contained-process-harness-runner.test.ps1` (60s), `windows-contained-process.contract.test.ps1` (60s) |
| `Evidence` | `contained-process-evidence.test.ps1` |
| `Integration` | `windows-contained-process.integration.test.ps1` |
| `All` | all six scripts in the order above |

Evidence has a fixed 60-second deadline and integration has a fixed 120-second deadline. First create `contained-process-harness-runner.test.ps1` and confirm it fails because its module is absent. Its fixtures must prove natural exit-code capture, argument paths containing spaces, a child-observed working directory equal to the canonical repository `desktop` directory, asynchronous draining beyond the pipe buffer, output beyond the per-stream cap without deadlock or unbounded file growth, a fixed short timeout, one nested descendant holding a case-local file open, bounded whole-tree kill, and single disposal. The timeout case must finish within its deadline plus one shared ten-second cleanup budget and must prove the descendant releases the case-local handle. The test also performs an AST check that the module contains no `GetProcessById`, `Get-Process`, CIM, WMI, PID-directed `Stop-Process`, retry loop, or import of the production smoke helper.

Implement runner-only `Invoke-HarnessSuiteWithDeadline` in `contained-process-harness-runner.psm1` and import it only from the canonical script and its focused test. Do not import `nsis-smoke-helpers.psm1` and do not call `Start-YapContainedProcess` or `Invoke-YapContainedProcessWithDeadline`. This watchdog is intentionally not a fourth consumer of the production boundary.

For each suite, construct `System.Diagnostics.ProcessStartInfo` with the supplied absolute executable, `WorkingDirectory` set explicitly to the validated canonical repository `desktop` directory, `UseShellExecute=false`, stdout/stderr redirection, no window, and exact arguments added separately through `ArgumentList`: `-NoProfile`, `-NonInteractive`, `-ExecutionPolicy`, `Bypass`, `-File`, and the absolute suite path. Retain the one `System.Diagnostics.Process` instance returned by `Start()`; never reacquire it from its PID or query CIM. Before waiting, start one asynchronous collector per stream. Each collector continuously drains to EOF, retains at most 1,048,576 UTF-8 bytes, discards/counts additional bytes, and writes one fixed truncation marker; it never uses `ReadToEndAsync()` or stores the discarded tail in memory/disk.

Wait on the retained process for the suite's fixed runtime deadline. When that wait ends, start one monotonic ten-second cleanup budget shared by all remaining work. On timeout, call `Kill(entireProcessTree: true)` exactly once. Under the single remaining cleanup budget, await the retained process-exit task and `Task.WhenAll(stdoutCollector, stderrCollector)` together; do not give each operation a fresh ten seconds. A timeout always fails the suite. A thrown kill, unsignaled retained process, incomplete collector, or exceeded shared budget sets runner cleanup proof to false. Natural completion still requires the retained process to be signaled and both bounded collectors to reach EOF within that same cleanup budget before recording its exit code.

Write only the bounded unsanitized stream prefixes plus fixed truncation markers to `private`, dispose the retained process once, and return a scalar runner report. Stop after the first failed suite and never retry it. `cleanup.proven` in the harness result means only that every started suite process reached this bounded runner completion path; it never authorizes installer-owned mutations or substitutes for the Job-quiescence evidence asserted inside the integration suite. The explicit `-NestedChild` fixture from Task 4 creates the outer-Job case; the canonical runner does not create a Job.

- [ ] **Step 6: Publish only sanitized artifact evidence**

After each suite, sanitize its private stdout/stderr through the tested module into bounded files under `artifact` and append only its scalar report to an in-memory result list. Never echo raw child output or an unsanitized exception to the Actions console. After the selected sequence completes or stops at its first failure, write `artifact/result.json.tmp` exactly once with no BOM, flush it, and rename without overwrite exactly once to `artifact/result.json`. A top-level catch must convert its error to the same sanitized scalar shape and reach that one publisher before the script exits non-zero; it must not attempt a second publication. Schema version 1 is:

```json
{
  "schemaVersion": 1,
  "runtimeLabel": "pinned-7.4.17",
  "runtime": {
    "fileName": "pwsh.exe",
    "canonicalPathSha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "locationClass": "runner-temp",
    "psEdition": "Core",
    "version": "7.4.17"
  },
  "status": "passed",
  "parsedFileCount": 0,
  "suites": [
    {
      "name": "windows-contained-process.integration.test.ps1",
      "status": "passed",
      "exitCode": 0,
      "elapsedMilliseconds": 0,
      "stdoutFile": "artifact/windows-contained-process.integration.test.stdout.log",
      "stderrFile": "artifact/windows-contained-process.integration.test.stderr.log"
    }
  ],
  "cleanup": { "proven": true, "retainedPaths": [] },
  "errors": [],
  "startedAtUtc": "2026-07-13T18:00:00.0000000Z",
  "finishedAtUtc": "2026-07-13T18:00:01.0000000Z"
}
```

The hash and timestamps above demonstrate shape only; implementation records observed values. Errors may contain stage, native error code, nonce, PID, and tokenized expected/observed executable identity. `retainedPaths` contains path-identity objects, not raw paths. The artifact subtree must contain no environment values, command lines, credentials, usernames, transcript/audio data, raw file contents, or nonce temporary files. After publication, run the same adversarial secret/path scan used by the redaction test across every artifact file. Exit non-zero only after publishing a sanitized failed result.

- [ ] **Step 7: Run the canonical entrypoint locally and return the policy contract to GREEN**

Resolve paths from `desktop`, but deliberately launch the entrypoint while the caller is in the system temp directory:

```powershell
$runtime = [IO.Path]::GetFullPath([Environment]::ProcessPath)
$desktopRoot = [IO.Path]::GetFullPath($PWD.Path)
$entrypoint = [IO.Path]::GetFullPath((Join-Path $desktopRoot "tests/scripts/run-contained-process-harness.ps1"))
$resultRoot = [IO.Path]::GetFullPath("./tests/results/contained-process-harness/local-current-$([Guid]::NewGuid().ToString('N'))")
Push-Location ([IO.Path]::GetTempPath())
try {
  & $runtime -NoProfile -NonInteractive -ExecutionPolicy Bypass `
    -File $entrypoint `
    -PowerShellExecutable $runtime `
    -RuntimeLabel "local-current" `
    -ResultRoot $resultRoot `
    -Suite Contract
  if ($LASTEXITCODE -ne 0) { throw "Canonical contained-process harness failed." }
} finally {
  Pop-Location
}
pnpm test:release-contract
```

Expected: the focused Contract selector and policy contract both exit 0. Inspect `artifact/result.json` and the sanitized logs; prove the private logs are outside the artifact subtree and no prohibited synthetic value survived. Task 6 already proved evidence/integration behavior; Task 9 is the only complete `All` phase gate.

- [ ] **Step 8: Commit the ownership split**

```powershell
git add desktop/tests/scripts/contained-process-redaction.psm1 `
  desktop/tests/scripts/contained-process-redaction.test.ps1 `
  desktop/tests/scripts/contained-process-harness-runner.psm1 `
  desktop/tests/scripts/contained-process-harness-runner.test.ps1 `
  desktop/tests/scripts/run-contained-process-harness.ps1 `
  desktop/tests/scripts/release-evidence.contract.mjs
git commit -m "test: separate process integration from release policy"
```

---

### Task 8: Give The Boundary Its Own CI Job And Release Gates

**Files:**
- Modify: `.github/workflows/ci.yml:13-126`
- Modify: `.github/workflows/nsis-smoke.yml:20-82`
- Modify: `.github/workflows/release.yml:75-184`
- Modify: `desktop/tests/scripts/release-evidence.contract.mjs`

- [ ] **Step 1: Add RED workflow-ownership contracts**

Extend `release-evidence.contract.mjs` to parse all three workflows and require:

- CI job id `windows-process-harness` with display name exactly `Windows process harness`, `windows-latest`, and `timeout-minutes: 25`;
- no job/path conditional and no retry or matrix strategy on that job;
- capture of hosted current PowerShell as named-step outputs before extracting 7.4.17, with no custom runtime identity stored in mutable `GITHUB_ENV`;
- a named `install_pinned` step with `timeout-minutes: 5` and `continue-on-error: true`, so a stalled download/extraction cannot consume the whole job or skip either lane;
- pinned version `7.4.17` and archive SHA-256 `266479A93B82CD0DC0F043419388FD4A738A51082821C301FFF497212FAF6760`;
- no write of the pinned runtime directory to `GITHUB_PATH`;
- exactly two CI calls to the canonical entrypoint, pinned first and captured-current second, both with absolute runtime arguments and distinct result roots;
- named `pinned_lane`/`current_lane` steps use `continue-on-error: true`, the current lane uses `if: always()`, and one final `if: always()` aggregate step fails unless both recorded step outcomes are `success`;
- zero contained-process harness calls in the frontend and native-WDIO jobs;
- exactly one captured-current harness call before installer smoke in each scheduled/release job;
- a dedicated-job upload guarded by the two lane outcomes and separate `if: failure()` uploads in scheduled/release, all limited to each nonce run's `artifact/result.json` and sanitized `artifact/*.stdout.log`/`artifact/*.stderr.log`, with seven-day retention;
- an adversarial policy fixture proving raw `private` logs and synthetic username/token/command-line strings cannot match an artifact upload path;
- no workflow copies the individual contract/evidence/integration script invocation list.

Run `pnpm test:release-contract`. Expected: RED against the old workflow ownership.

- [ ] **Step 2: Move the compatibility runtime out of `frontend`**

Remove `POWERSHELL_74_VERSION`, `POWERSHELL_74_SHA256`, the pinned-runtime install, `GITHUB_PATH` mutation, and focused helper-suite step from `frontend`. Keep the generic Core 7.4 minimum check because frontend-owned repository scripts still require it. Do not change frontend dependency caches or its unit/build/E2E commands.

- [ ] **Step 3: Add the dedicated always-reported CI job**

Add this job shape to `ci.yml`:

```yaml
  windows-process-harness:
    name: Windows process harness
    runs-on: windows-latest
    timeout-minutes: 25
    env:
      POWERSHELL_74_SHA256: 266479A93B82CD0DC0F043419388FD4A738A51082821C301FFF497212FAF6760
      POWERSHELL_74_VERSION: 7.4.17
      POWERSHELL_TELEMETRY_OPTOUT: "1"
    defaults:
      run:
        shell: pwsh
        working-directory: desktop
```

Its steps must:

1. check out with the existing full-SHA `actions/checkout` pin;
2. in a named `capture-hosted-pwsh` step, write `[Environment]::ProcessPath`, `PSEdition`, and exact hosted version to `GITHUB_OUTPUT` before any download;
3. in named step `install_pinned`, use `timeout-minutes: 5` and `continue-on-error: true`; download the official 7.4.17 x64 zip, verify the exact SHA-256, extract it to a separate `RUNNER_TEMP` directory without modifying `PATH`, probe the extracted absolute path as Core exactly 7.4.17, and expose that path through `GITHUB_OUTPUT`, not `GITHUB_ENV`;
4. run the named pinned lane with `if: always()` and `continue-on-error: true`, invoke the canonical entrypoint under the pinned absolute path with that same path passed as `PowerShellExecutable`, label `pinned-7.4.17`, and a unique result root, and fail the step if installation/probe output is missing or its harness exits non-zero;
5. run the named hosted-current lane with `if: always()` and `continue-on-error: true` so setup or pinned-lane failure cannot skip current-runtime evidence; pass the three immutable capture outputs through step-level `env`, immediately re-probe path/edition/version, require exact equality, and invoke the canonical entrypoint under that captured absolute path with label `hosted-current` and a separate result root;
6. when `steps.pinned_lane.outcome != 'success' || steps.current_lane.outcome != 'success'`, use `if: always()` and the existing full-SHA `upload-artifact` pin to upload only each nonce run's `artifact/result.json`, `artifact/*.stdout.log`, and `artifact/*.stderr.log`, with `retention-days: 7`; reject directory-wide `artifact/**`/`artifact` paths and never upload `private`;
7. in the final `if: always()` aggregate step, read both named step outcomes and throw unless each equals `success`.

Give the capture and install steps stable IDs as well. The pinned lane treats a failed/missing install output as its own recorded failure; neither lane is skipped by an earlier failure. Every absolute-runtime invocation checks `$LASTEXITCODE` immediately. The two `All` lanes have a combined declared upper bound of 900 seconds (390 seconds of suite runtime plus six shared ten-second cleanup budgets per lane), so the 25-minute job ceiling leaves setup, parsing, publication, upload, and aggregate-step margin. The second lane runs even when its version is also 7.4.x; it proves that the pinned extraction did not contaminate hosted resolution. The aggregate step makes the job fail if either lane fails while preserving both outcomes and the sanitized evidence upload.

- [ ] **Step 4: Wire scheduled smoke to the captured current runtime once**

In `nsis-smoke.yml`, capture the absolute hosted `pwsh.exe` path, edition, and version as named-step outputs in the existing minimum-runtime step. After the NSIS build and before `smoke-nsis-production-delete.ps1`, pass those outputs through step-level `env`, re-probe all three values immediately, and invoke the canonical harness once with `RuntimeLabel=nsis-hosted-current` and a workflow-unique result root. Add a distinct failure-only, seven-day artifact containing only `artifact/result.json`, `artifact/*.stdout.log`, and `artifact/*.stderr.log`; reject a whole-directory wildcard. Do not install 7.4.17 or run a second lane in this release-boundary workflow.

- [ ] **Step 5: Wire release to the captured current runtime without breaking artifact sealing**

In `release.yml`'s `build-nsis` job, capture the absolute hosted runtime, edition, and version as named-step outputs before build steps. Preserve the existing order through immutable context creation, provenance, build, and exact artifact seal. Insert one canonical harness call after sealing and immediately before `Smoke the exact NSIS release artifact`; pass the capture through step-level `env`, re-probe it immediately, and use `RuntimeLabel=release-hosted-current`. Keep the sealed hash argument and all later evidence-binding/publishing steps unchanged. Add a distinct failure-only, seven-day upload limited to `artifact/result.json`, `artifact/*.stdout.log`, and `artifact/*.stderr.log`; reject a whole-directory wildcard and do not add harness files to the immutable release payload.

- [ ] **Step 6: Run only the affected policy and YAML checks**

```powershell
pnpm test:release-contract
git diff --check
```

Expected: both commands exit 0. The Node policy contract performs the authoritative YAML parse and workflow-shape assertions. Task 8 changes only workflow ownership, so do not rerun the native harness here; Task 7 already proved the entrypoint and Task 9 owns the complete pinned/current Phase 3 run.

- [ ] **Step 7: Commit the independent CI boundary**

```powershell
git add .github/workflows/ci.yml `
  .github/workflows/nsis-smoke.yml `
  .github/workflows/release.yml `
  desktop/tests/scripts/release-evidence.contract.mjs
git commit -m "ci: add the Windows process harness gate"
```

---

### Task 9: Reconcile Documentation And Run The Complete Phase 3 Gate Once

**Files:**
- Modify: `docs/specs/testing-strategy.md`
- Modify: `README.md`
- Modify: `docs/README.md`
- Modify: `docs/ADR-IMPLEMENTATION-STATUS.md`
- Modify: `docs/superpowers/specs/2026-07-13-contained-process-boundary-design.md`
- Modify: `docs/superpowers/plans/2026-07-13-ci-actions-cache-hardening.md:108-141`

- [ ] **Step 1: Reconcile docs with executable ownership before the full gate**

Document the final facts, not planned claims:

- retained native handle + pre-resume Job assignment is authoritative for installer/app/uninstaller;
- CIM is a post-disposal executable-path audit only;
- native evidence is nonce-bound corroboration only;
- `test:release-contract` is policy-only;
- the canonical harness command requires an explicit absolute runtime path and runs under pinned 7.4.17 plus hosted current in CI;
- the separately named required context is `Windows process harness`;
- failed cleanup retains paths/evidence and blocks mutations;
- no GB10/server deployment rerun is part of this Windows-only change; the local server unit and connector contracts still run in the phase gate.

After every local command in this task passes, set the design status only to `Implemented locally on 2026-07-13; hosted verification pending`. Add `## 16. Implementation Evidence` with a `Local verification` subsection containing only the commands/counts/runtime identities observed in this task and a `Hosted and governance verification` subsection stating explicitly that PR, merged-main, ruleset, Actions-policy, cache, and open-PR closure are still pending. Update ADR implementation status without changing unrelated ADR scores. In the prior CI/cache plan, preserve the historical PR #52/post-main failure narrative, reconcile statements already disproven by current repository truth, and leave every PR/main/cache/ruleset/action-policy closure checkbox pending. Task 9 must not claim evidence that Task 10 has not gathered. Only Task 10's final evidence PR may mark the design fully implemented and verified and complete those imported operational boxes.

- [ ] **Step 2: Install locked dependencies and run the policy/frontend gate**

From `desktop`:

```powershell
pnpm install --frozen-lockfile
pnpm audit --audit-level high
pnpm test:release-contract
pnpm test
pnpm build
pnpm exec playwright install chromium
pnpm test:e2e
node ./tests/scripts/assert-third-party-provenance.mjs
```

Expected: every command exits 0. Record exact test counts and durations from output; do not copy older counts into the docs.

- [ ] **Step 3: Run the canonical harness under current and pinned 7.4.17**

Use an isolated local download and the same official checksum as CI:

```powershell
$currentRuntime = [IO.Path]::GetFullPath([Environment]::ProcessPath)
$version = "7.4.17"
$expectedHash = "266479A93B82CD0DC0F043419388FD4A738A51082821C301FFF497212FAF6760"
$tempBase = [IO.Path]::GetFullPath($env:TEMP)
$nonce = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(16)).ToLowerInvariant()
$tempRoot = [IO.Path]::GetFullPath((Join-Path $tempBase "yap-powershell-$nonce"))
$sentinel = Join-Path $tempRoot ".yap-local-powershell-v1"
$tempOwned = $false
if (Test-Path -LiteralPath $tempRoot) { throw "Fresh PowerShell temp root already exists." }
[IO.Directory]::CreateDirectory($tempRoot) | Out-Null
$sentinelStream = [IO.File]::Open($sentinel, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
try {
  $sentinelBytes = [Text.Encoding]::UTF8.GetBytes($nonce)
  $sentinelStream.Write($sentinelBytes, 0, $sentinelBytes.Length)
  $sentinelStream.Flush($true)
} finally {
  $sentinelStream.Dispose()
}
$tempOwned = $true
$laneError = $null
$tempCleanupError = $null

try {
  $archive = Join-Path $tempRoot "PowerShell-$version-win-x64.zip"
  $runtimeRoot = Join-Path $tempRoot "runtime"
  Invoke-WebRequest "https://github.com/PowerShell/PowerShell/releases/download/v$version/PowerShell-$version-win-x64.zip" -OutFile $archive
  if ((Get-FileHash $archive -Algorithm SHA256).Hash -cne $expectedHash) { throw "PowerShell 7.4.17 archive hash mismatch." }
  Expand-Archive -LiteralPath $archive -DestinationPath $runtimeRoot
  $pinnedRuntime = [IO.Path]::GetFullPath((Join-Path $runtimeRoot "pwsh.exe"))

  foreach ($lane in @(
    @{ Path = $pinnedRuntime; Label = "local-pinned-7.4.17" },
    @{ Path = $currentRuntime; Label = "local-current" }
  )) {
    $resultRoot = [IO.Path]::GetFullPath("./tests/results/contained-process-harness/$($lane.Label)-$([Guid]::NewGuid().ToString('N'))")
    & $lane.Path -NoProfile -NonInteractive -ExecutionPolicy Bypass `
      -File ./tests/scripts/run-contained-process-harness.ps1 `
      -PowerShellExecutable $lane.Path `
      -RuntimeLabel $lane.Label `
      -ResultRoot $resultRoot `
      -Suite All
    if ($LASTEXITCODE -ne 0) { throw "Contained-process lane $($lane.Label) failed." }
  }
}
catch {
  $laneError = $_
}
finally {
  if ($tempOwned) {
    try {
      $strictPrefix = $tempBase.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
      if (-not $tempRoot.StartsWith($strictPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw "Refusing non-child PowerShell temp cleanup." }
      if (-not (Test-Path -LiteralPath $sentinel -PathType Leaf)) { throw "Refusing PowerShell temp cleanup without sentinel." }
      if (([IO.File]::GetAttributes($tempRoot) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Refusing reparse-point PowerShell temp cleanup." }
      if (([IO.File]::GetAttributes($sentinel) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Refusing reparse-point PowerShell sentinel." }
      $sentinelRead = [IO.File]::Open($sentinel, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
      try {
        $sentinelReader = [IO.StreamReader]::new($sentinelRead, [Text.Encoding]::UTF8, $false, 1024, $true)
        try { $observedNonce = $sentinelReader.ReadToEnd() } finally { $sentinelReader.Dispose() }
      } finally {
        $sentinelRead.Dispose()
      }
      if ($observedNonce -cne $nonce) { throw "Refusing PowerShell temp cleanup with foreign sentinel." }
      Remove-Item -LiteralPath $tempRoot -Recurse -Force
    } catch {
      $tempCleanupError = $_
    }
  }
}
if ($null -ne $laneError -and $null -ne $tempCleanupError) { throw [AggregateException]::new("PowerShell lanes and temp cleanup failed.", [Exception[]]@($laneError.Exception, $tempCleanupError.Exception)) }
if ($null -ne $laneError) { throw $laneError }
if ($null -ne $tempCleanupError) { throw $tempCleanupError }
```

Expected: both lanes exit 0 with distinct atomic result records and no residual process/path lock.

- [ ] **Step 4: Run Rust, server, and live connector contracts locally**

From the repository root:

```powershell
cargo fmt --all --check --manifest-path ./desktop/src-tauri/Cargo.toml
cargo clippy --locked --all-targets --manifest-path ./desktop/src-tauri/Cargo.toml -- -D warnings
cargo test --locked --manifest-path ./desktop/src-tauri/Cargo.toml
$repoRoot = [IO.Path]::GetFullPath($PWD.Path)
$serverSource = [IO.Path]::GetFullPath((Join-Path $repoRoot "server/src"))
$python = [IO.Path]::GetFullPath((Get-Command python -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source)
$environmentNames = @("PYTHONPATH", "YAP_SERVER_HOST", "YAP_SERVER_PORT", "YAP_TEST_SERVER_URL")
$previousEnvironment = @{}
foreach ($name in $environmentNames) {
  $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}
$server = $null
$serverStarted = $false
$bodyError = $null
$cleanupError = $null
try {
  [Environment]::SetEnvironmentVariable("PYTHONPATH", $serverSource, "Process")
  & $python -m unittest discover -s server/tests -p "test_*.py"
  if ($LASTEXITCODE -ne 0) { throw "Server unit and contract tests failed." }

  $reservation = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
  $reservation.Start()
  try {
    $port = ([Net.IPEndPoint]$reservation.LocalEndpoint).Port
  } finally {
    $reservation.Stop()
  }
  [Environment]::SetEnvironmentVariable("YAP_SERVER_HOST", "127.0.0.1", "Process")
  [Environment]::SetEnvironmentVariable("YAP_SERVER_PORT", $port.ToString([Globalization.CultureInfo]::InvariantCulture), "Process")
  $serverInfo = [Diagnostics.ProcessStartInfo]::new()
  $serverInfo.FileName = $python
  $serverInfo.UseShellExecute = $false
  $serverInfo.CreateNoWindow = $true
  $serverInfo.ArgumentList.Add("-m")
  $serverInfo.ArgumentList.Add("yap_server")
  $server = [Diagnostics.Process]::new()
  $server.StartInfo = $serverInfo
  if (-not $server.Start()) { throw "Local Yap server process did not start." }
  $serverStarted = $true
  $ready = $false
  for ($attempt = 0; $attempt -lt 40; $attempt++) {
    if ($server.HasExited) { throw "Local Yap server exited before readiness." }
    $owners = @(Get-NetTCPConnection -State Listen -LocalAddress "127.0.0.1" -LocalPort $port -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess -Unique)
    if ($owners.Count -gt 0) {
      if ($owners.Count -ne 1 -or [int]$owners[0] -ne $server.Id) { throw "Reserved server port is owned by another process." }
      try {
        Invoke-RestMethod "http://127.0.0.1:$port/v1/health" | Out-Null
        $ready = $true
        break
      } catch {}
    }
    Start-Sleep -Milliseconds 250
  }
  if (-not $ready) { throw "Local Yap server health did not become ready." }
  [Environment]::SetEnvironmentVariable("YAP_TEST_SERVER_URL", "http://127.0.0.1:$port", "Process")
  cargo test --locked --manifest-path ./desktop/src-tauri/Cargo.toml --test server_connector
  if ($LASTEXITCODE -ne 0) { throw "Server connector integration failed." }
} catch {
  $bodyError = $_
} finally {
  try {
    if ($serverStarted -and -not $server.HasExited) { $server.Kill($true) }
    if ($serverStarted -and -not $server.WaitForExit(10000)) { throw "Local Yap server did not exit within cleanup deadline." }
  } catch {
    $cleanupError = $_
  } finally {
    if ($null -ne $server) { $server.Dispose() }
    foreach ($name in $environmentNames) {
      [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], "Process")
    }
  }
}
if ($null -ne $bodyError -and $null -ne $cleanupError) { throw [AggregateException]::new("Server connector body and cleanup failed.", [Exception[]]@($bodyError.Exception, $cleanupError.Exception)) }
if ($null -ne $bodyError) { throw $bodyError }
if ($null -ne $cleanupError) { throw $cleanupError }
```

Expected: formatting, warnings-denied Clippy, all Rust tests, all Python tests, and the real loopback connector pass. This intentionally does not mutate or deploy to the plugged-in GB10 server.

- [ ] **Step 5: Run native WDIO and both installer policies**

From `desktop`:

```powershell
pnpm test:desktop:build
pnpm exec wdio run ./tests/wdio.required.conf.ts
pnpm build:nsis:test
pnpm test:nsis:local
pnpm test:nsis:test-delete
```

Expected: every command exits 0; the app process is owned by the lease; default uninstall preserves the test data namespace; explicit deletion removes only sentinel-authorized data; no installer-owned process or path lock remains.

- [ ] **Step 6: Record observed evidence, validate the complete diff, and commit docs**

Update the listed docs with exact command results, test counts, runtime identities, and any deliberate skip. Then run:

```powershell
git diff --check
git status --short
rg -n "KillOnCloseJob|Test-ProcessAlive|Test-ProcessIdentityAlive|Get-ProcessTreeIds|ConvertTo-ProcessCreationIdentity|Normalize-ProcessCreationIdentity|Stop-VerifiedProcessHandle|Update-TrackedProcessIds|Get-TrackedProcessDepth|ConvertTo-SerializableIdentityMap|Stop-TrackedProcessesBounded|Stop-ProcessTreeBounded|Start-ProcessWithEnvironment|Start-JobContainedProcess|Stop-JobContainedProcess|Invoke-ProcessWithDeadline|Assert-ProcessSurvives|Add-TrackedProcess|trackedProcessIds|treeProcessIds|DiscoveredProcessIds|TerminatedProcessIds|ResidualProcessIds|ReusedProcessIds|ProcessIdentityById|appProcess(Id|Identity|StartTime)?" `
  desktop/tests/scripts/nsis-smoke-helpers.psm1 `
  desktop/tests/scripts/smoke-nsis.ps1
```

Expected: `git diff --check` exits 0; status contains only intended files; `rg` finds no match in the two production smoke files. Negative contract fixtures may retain removed names only as forbidden-symbol assertions; do not weaken those tests to make a repository-wide text search empty.

```powershell
git add docs/specs/testing-strategy.md README.md docs/README.md `
  docs/ADR-IMPLEMENTATION-STATUS.md `
  docs/superpowers/specs/2026-07-13-contained-process-boundary-design.md `
  docs/superpowers/plans/2026-07-13-ci-actions-cache-hardening.md
git commit -m "docs: record the contained process verification boundary"
```

---

### Task 10: Review, Merge Green, Harden Governance, And Halt Phase 3

**Files:**
- Modify after live read-back: `docs/superpowers/plans/2026-07-13-ci-actions-cache-hardening.md`
- Modify after live read-back: `docs/superpowers/specs/2026-07-13-contained-process-boundary-design.md`

The ruleset, full-SHA Actions policy, cache inventory, and open-PR operations in this task are imported outstanding obligations from the separately approved `2026-07-13-ci-actions-cache-hardening.md` plan. They are sequenced here to close Phase 3 after the process boundary is green; they are not new runtime scope or consequences of the Windows boundary. The only boundary-specific governance change is adding `Windows process harness` to the stable required-context set.

- [ ] **Step 1: Run independent code and security reviews**

Request two fresh reviews over `origin/main...HEAD`:

1. Native/process reviewer: retained-handle identity, SafeHandle lifetime, pre-resume assignment, quoting/environment fidelity, nested Jobs, bounded waits, race behavior, fail-closed cleanup.
2. CI/release reviewer: test ownership, exact runtime isolation, redaction, workflow ordering, action pins, required-context stability, release sealing, and no hidden integration.

Classify findings as Critical, Important, or Minor. Resolve every Critical/Important finding, rerun only affected focused checks, then rerun the complete local Phase 3 gate once if any fix changes native lifecycle, cleanup authorization, canonical harness behavior, or workflow ordering. Record `0 Critical / 0 Important remaining` before opening the PR.

- [ ] **Step 2: Push a focused PR and merge only the checked head SHA**

```powershell
git status --short
git log --oneline origin/main..HEAD
git push -u origin fix/contained-windows-process-boundary
gh pr create `
  --base main `
  --head fix/contained-windows-process-boundary `
  --title "fix: make Windows process containment authoritative" `
  --body "Implements the approved retained-handle NSIS smoke boundary, separates process integration from release policy, and adds the stable Windows process harness gate. Local Phase 3 verification and independent native/release reviews are recorded in the committed docs."
```

Capture the PR number, exact head SHA, and current base SHA from `gh pr view --json number,headRefOid,baseRefOid,isDraft,mergeable,mergeStateStatus`. Query check runs by that head SHA. The following six names are the required subset; unrelated extra checks are allowed:

```text
frontend
rust
server
Native WDIO smoke (required, no hardware)
Windows process harness
CodeQL
```

Require exactly one check-run identity for each stable name. Accept only `success` for the five Actions contexts and `success` or GitHub's successful `neutral` conclusion for the stable `CodeQL` aggregate. Separately query every emitted CodeQL `Analyze (...)` check and require it to be green. A missing stable context, ambiguous duplicate, queued/in-progress state, cancellation, failure, or stale SHA blocks merge; an omitted matrix-internal `Analyze (...)` job does not.

Immediately before merge, re-read the PR and remote `main`. Require the PR to remain non-draft and mergeable, its head SHA to equal the checked SHA, and its merge state to show it is not behind or conflicted. If `main` advanced or the branch is behind, update/rebase the PR branch, capture the new head SHA, and restart every hosted check above. Then merge with the checked SHA bound into the command:

```powershell
gh pr merge $prNumber --squash --delete-branch --match-head-commit $prHeadSha
```

Do not bypass, admin-merge, merge red, or rely on a check from an older head.

- [ ] **Step 3: Verify the exact merged SHA on `main` before any governance mutation**

Resolve the squash commit SHA from the merged PR and require it to equal the current `origin/main` tip. Wait for its `CI` push run and require successful conclusions for the five main Actions contexts: `frontend`, `rust`, `server`, `Native WDIO smoke (required, no hardware)`, and `Windows process harness`. Query CodeQL analyses for `ref=refs/heads/main`, filter to that exact commit SHA, and require the non-empty category set to equal `/language:actions`, `/language:javascript-typescript`, `/language:python`, and `/language:rust`; each exact-SHA record must be complete with an empty error. Zero rows, a missing/extra category, an older SHA, or a non-main ref fails verification.

- [ ] **Step 4: Preflight caches and every open PR before enabling protection**

List all Actions caches with id, key, ref, size, creation, and last-access timestamps. Preserve the canonical pnpm, Playwright, Cargo download caches and GitHub-managed CodeQL overlay caches. Delete nothing unless a cache is conclusively obsolete and its canonical replacement from the successful exact merged SHA exists. Record the actual before/after inventory; do not repeat the old 9.45-GiB baseline as current truth.

List every open PR with number, author, head SHA, draft state, mergeability, and all check runs. For every existing PR, prove all six stable contexts are emitted; require the newly introduced `Windows process harness` to be green and apply the documented success/neutral rule to CodeQL. Record the conclusions of the other four Actions checks and every emitted analysis rather than pretending an unrelated pre-existing failure passed. If a PR lacks `Windows process harness` because its head predates the new `main`, update its branch, capture its new head, and wait for the context before changing the ruleset. An unrelated pre-existing failure may remain documented, but no PR may be stranded solely by a missing new context. If an open branch cannot safely be updated or cannot emit the new stable context, stop before governance mutation. Do not merge unrelated dependency PRs.

- [ ] **Step 5: Derive integration IDs and update ruleset `18846727` with explicit drift checks**

From the successful process-boundary PR head, map each stable context to exactly one observed GitHub App integration ID. Reject a missing, null, or ambiguous mapping. Historical evidence suggests Actions app `15368` and CodeQL app `57789`, but use only newly observed IDs.

Read ruleset `18846727` immediately before mutation. The expected 2026-07-13 baseline is name `No touchtg`, target `branch`, enforcement `active`, empty bypass actors, empty `ref_name.include`/`exclude`, and exactly the parameterless `deletion`, `non_fast_forward`, and `required_linear_history` rules. Deep-compare that complete baseline first. If any field or rule has drifted, stop and reconcile rather than overwriting it.

Build one explicit replacement payload containing every writable top-level field:

```powershell
$stableContexts = @(
  "frontend",
  "rust",
  "server",
  "Native WDIO smoke (required, no hardware)",
  "Windows process harness",
  "CodeQL"
)
$rules = @(
  [ordered]@{ type = "deletion" },
  [ordered]@{ type = "non_fast_forward" },
  [ordered]@{ type = "required_linear_history" },
  [ordered]@{
    type = "pull_request"
    parameters = [ordered]@{
      allowed_merge_methods = @("squash", "rebase")
      dismiss_stale_reviews_on_push = $false
      require_code_owner_review = $false
      require_last_push_approval = $false
      required_approving_review_count = 0
      required_review_thread_resolution = $true
    }
  },
  [ordered]@{
    type = "required_status_checks"
    parameters = [ordered]@{
      do_not_enforce_on_create = $false
      strict_required_status_checks_policy = $true
      required_status_checks = @(
        $stableContexts | ForEach-Object {
          [ordered]@{ context = $_; integration_id = [int]$observedIntegrationIds[$_] }
        }
      )
    }
  }
)
$payload = [ordered]@{
  name = [string]$liveRuleset.name
  target = [string]$liveRuleset.target
  enforcement = [string]$liveRuleset.enforcement
  bypass_actors = @($liveRuleset.bypass_actors | ForEach-Object {
    [ordered]@{ actor_id = [int]$_.actor_id; actor_type = [string]$_.actor_type; bypass_mode = [string]$_.bypass_mode }
  })
  conditions = [ordered]@{
    ref_name = [ordered]@{ include = @("~DEFAULT_BRANCH"); exclude = @() }
  }
  rules = $rules
}
```

The ruleset update endpoint does not document compare-and-swap support for unsafe requests, so do not pretend a weak ETag/`If-Match` header makes the write atomic. Use a short repository-settings maintenance window with no concurrent admin changes, perform a second full baseline read immediately before `PUT`, require it to equal the first, and send the complete payload once. Immediately read both `GET /repos/mcnatg1/yap/rulesets/18846727` and `GET /repos/mcnatg1/yap/rules/branches/main`. Normalize and deep-compare `name`, `target`, `enforcement`, every bypass actor field, both condition arrays, all five rule types, every pull-request parameter, strict/create flags, and the full sorted context-to-integration mapping. The effective-main response must attribute equivalent rules to ruleset `18846727`. Any pre-write drift or post-write semantic mismatch is terminal; preserve both JSON snapshots, do not issue a blind second PUT, and reconcile the live state before continuing. Use the [repository rulesets API](https://docs.github.com/en/rest/repos/rules#update-a-repository-ruleset) contract rather than a partial hand-built request.

- [ ] **Step 6: Enable full-SHA Actions enforcement and verify it**

Parse every tracked `.github/workflows/*.yml`/`.yaml` and require every `uses:` value to end in a reviewed 40-character hexadecimal commit SHA. Read `GET /repos/mcnatg1/yap/actions/permissions` immediately before mutation; preserve its observed `enabled` and `allowed_actions` values while setting `sha_pinning_required=true` through one complete `PUT`. Read the endpoint back and require all three fields to equal the intended payload. Unexpected policy drift or any unpinned action blocks this step.

- [ ] **Step 7: Land the evidence-only PR and verify its exact merged SHA**

Replace the pending subsection in design Section 16 and update the prior CI/cache plan with exact implementation PR number/head/squash SHA, workflow run IDs, observed integration IDs, main analysis evidence, ruleset deep read-back, action-policy read-back, cache inventory, open-PR audit, and the private-repository caveat. At this point the design status may say it is implemented and verified through the exact implementation squash SHA and recorded governance read-back; it must not claim the still-future evidence-PR merge run. The code/workflow contracts are visibility-independent, but GitHub Free rulesets are available for public repositories; a user-owned private repository needs GitHub Pro or an appropriate Team/Enterprise ownership plan for equivalent enforcement. Before changing visibility later, re-check ruleset, CodeQL, Actions-minute, and runner availability.

Create the evidence branch from the freshly fetched default branch, never from the old implementation head:

```powershell
git fetch origin main
git switch -c docs/contained-process-boundary-evidence origin/main
pnpm --dir ./desktop test:release-contract
git add docs/superpowers/plans/2026-07-13-ci-actions-cache-hardening.md `
  docs/superpowers/specs/2026-07-13-contained-process-boundary-design.md
git diff --cached --check
git commit -m "docs: record Phase 3 boundary evidence"
git push -u origin docs/contained-process-boundary-evidence
```

Open a small PR and capture `number`, `headRefOid`, `baseRefOid`, `isDraft`, `mergeable`, and `mergeStateStatus`. Require the same six stable PR contexts plus every emitted analysis for that exact head. Immediately before merge, fetch/read remote `main` and re-read all six PR fields. Require the PR to remain non-draft and mergeable, its head to equal the checked SHA, its base SHA to equal current remote `main`, and its merge state to be clean rather than behind/conflicted. If the base advanced or the branch is behind, update/rebase the evidence branch, capture its new head/base, and restart all six stable plus emitted-analysis checks. Only then run:

```powershell
gh pr merge $evidencePrNumber --squash --delete-branch --match-head-commit $evidenceHeadSha
```

Do not weaken the new ruleset to land evidence. Fetch `origin/main` after merge, resolve the evidence PR's squash SHA, and require it to equal the fetched default-branch tip. Then wait for all five Actions contexts and the same exact four non-empty CodeQL categories on that final main commit, each complete and error-free. Finally re-read and deep-compare ruleset `18846727` and the Actions permissions endpoint once more. Record this final docs-only main verification in the Phase 3 completion report rather than creating an infinite chain of evidence-only PRs.

- [ ] **Step 8: Report Phase 3 complete and halt**

Only after the evidence PR's exact merged SHA is green and both governance read-backs still match may Phase 3 be reported complete. The imported operational boxes checked in the evidence PR must refer only to operations already performed; the final docs-only main verification is the report's postcondition, not a reason to create another documentation PR. Report the final local/hosted evidence, both merged SHAs, governance state, cache state, open-PR state, private-visibility caveat, and remaining roadmap. Then halt. Do not start client UX convergence, Freeflow/Meetily transplantation, broader server work, PowerShell-wide migration, HTTP/3, WebSocket, or later ADR phases until the user explicitly says `proceed`.

---

## Archived Enterprise Completion Definition (Non-Operative)

This plan is complete only when all of these are true:

- every approved NSIS smoke consumer has exactly one retained-handle lease owner;
- assignment and identity capture occur before the first resume;
- no PID/`StartTime`/CIM path owns termination;
- uncertain cleanup blocks every installer-owned mutation and preserves evidence;
- atomic evidence corroborates but never controls lifecycle;
- the policy contract launches no native integration;
- the canonical harness passes under exact 7.4.17 and captured hosted current PowerShell;
- the complete local Phase 3 gate, focused PR, and exact merged-main run are green;
- six stable contexts are required without pinning optional matrix-internal CodeQL jobs;
- ruleset, full-SHA Actions enforcement, caches, open PRs, and private-visibility caveat are read back and documented;
- Phase 3 is reported complete and work halts for explicit user approval.
