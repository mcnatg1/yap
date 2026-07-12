$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "nsis-smoke-helpers.psm1") -Force

function Assert-True([bool]$Condition, [string]$Message) {
  if (-not $Condition) { throw $Message }
}

function Assert-Throws([scriptblock]$Operation, [string]$Pattern) {
  try {
    & $Operation
  } catch {
    if ($_.Exception.Message -notmatch $Pattern) {
      throw "Expected error matching '$Pattern', received '$($_.Exception.Message)'."
    }
    return
  }
  throw "Expected operation to throw an error matching '$Pattern'."
}

function Assert-FileUnlocked([string]$Path, [string]$Message) {
  $stream = $null
  try {
    $stream = [System.IO.File]::Open(
      $Path,
      [System.IO.FileMode]::OpenOrCreate,
      [System.IO.FileAccess]::ReadWrite,
      [System.IO.FileShare]::None
    )
  } catch {
    throw "$Message $($_.Exception.Message)"
  } finally {
    if ($null -ne $stream) { $stream.Dispose() }
  }
}

$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$testRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-helper-test-$PID"
$externalRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-helper-external-$PID"
$processRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-helper-process-$PID"

try {
  Initialize-ValidatedTree -Root $tempRoot -Candidate $testRoot | Out-Null
  Initialize-ValidatedTree -Root $tempRoot -Candidate $externalRoot | Out-Null
  Initialize-ValidatedTree -Root $tempRoot -Candidate $processRoot | Out-Null

  Assert-True (Test-StrictChildPath -Root $tempRoot -Candidate $testRoot) "Expected strict child path."
  Assert-True (-not (Test-StrictChildPath -Root $testRoot -Candidate $testRoot)) "Root is not its own child."
  Assert-True (-not (Test-StrictChildPath -Root $testRoot -Candidate "$testRoot-sibling")) "Sibling prefix escaped containment."
  Assert-Throws { Get-ValidatedChildPath -Root $tempRoot -Token "..\escape" } "Unsafe path token"
  Assert-Throws { Get-ValidatedChildPath -Root $tempRoot -Token ("x" * 65) } "Unsafe path token"

  $hashFixture = Join-Path $testRoot "sha256.txt"
  [System.IO.File]::WriteAllText($hashFixture, "abc")
  Assert-True (
    (Get-Sha256Hex -Path $hashFixture) -ceq "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
  ) "Framework SHA-256 helper returned the wrong digest."

  $nsisRoot = Join-Path $testRoot "nsis-cache"
  $nsisBin = Join-Path $nsisRoot "Bin"
  New-Item -ItemType Directory -Force $nsisBin | Out-Null
  $nsisLauncher = Join-Path $nsisRoot "makensis.exe"
  $nsisCompiler = Join-Path $nsisBin "makensis.exe"
  [System.IO.File]::WriteAllBytes($nsisLauncher, [byte[]]@(1))
  [System.IO.File]::WriteAllBytes($nsisCompiler, [byte[]]@(2))
  $extraNsisDirectory = Join-Path $nsisRoot "unrelated-copy"
  New-Item -ItemType Directory -Force $extraNsisDirectory | Out-Null
  [System.IO.File]::WriteAllBytes((Join-Path $extraNsisDirectory "makensis.exe"), [byte[]]@(3))
  $nsisTools = Get-TauriNsisToolPaths -Root $nsisRoot
  Assert-True ($nsisTools.LauncherPath -ceq $nsisLauncher) "NSIS launcher path was not deterministic."
  Assert-True ($nsisTools.CompilerPath -ceq $nsisCompiler) "NSIS compiler path was not deterministic."
  Remove-Item -LiteralPath $nsisCompiler -Force -ErrorAction Stop
  Assert-Throws { Get-TauriNsisToolPaths -Root $nsisRoot } "compiler.*missing|missing.*compiler"
  [System.IO.File]::WriteAllBytes($nsisCompiler, [byte[]]@(2))

  [System.IO.Directory]::Delete($nsisBin, $true)
  New-Item -ItemType Junction -Path $nsisBin -Target $externalRoot | Out-Null
  Assert-Throws { Get-TauriNsisToolPaths -Root $nsisRoot } "Reparse point"
  [System.IO.Directory]::Delete($nsisBin, $false)

  $junction = Join-Path $testRoot "junction"
  New-Item -ItemType Junction -Path $junction -Target $externalRoot | Out-Null
  Assert-Throws { Assert-NoReparsePoints -Path $testRoot } "Reparse point"
  [System.IO.Directory]::Delete($junction, $false)
  Assert-True (Test-Path -LiteralPath $externalRoot -PathType Container) "Junction cleanup touched its target."

  $quickOut = Join-Path $processRoot "quick.out.log"
  $quickErr = Join-Path $processRoot "quick.err.log"
  $quick = Invoke-ProcessWithDeadline `
    -FilePath "cmd.exe" `
    -ArgumentList @("/d", "/c", "exit 7") `
    -TimeoutSeconds 5 `
    -StdoutPath $quickOut `
    -StderrPath $quickErr
  Assert-True ($quick.ExitCode -eq 7) "Deadline helper lost the process exit code."
  Assert-True ($quick.ProcessIds -contains $quick.ProcessId) "Deadline helper omitted its root process evidence."
  Assert-True ($quick.QuiescencePasses -ge 2) "Deadline helper did not verify process-tree quiescence."
  Assert-True (
    -not [string]::IsNullOrWhiteSpace(($quick | ConvertTo-Json -Depth 5))
  ) "Deadline helper returned evidence that Windows PowerShell cannot serialize."

  $slowSnapshotQuick = Invoke-ProcessWithDeadline `
    -FilePath "cmd.exe" `
    -ArgumentList @("/d", "/c", "exit 0") `
    -TimeoutSeconds 1 `
    -StdoutPath (Join-Path $processRoot "slow-snapshot-quick.out.log") `
    -StderrPath (Join-Path $processRoot "slow-snapshot-quick.err.log") `
    -QuiescenceTimeoutSeconds 0.2 `
    -SnapshotTimeoutSeconds 0.5 `
    -PollMilliseconds 1 `
    -SnapshotProviderScript "Start-Sleep -Milliseconds 300"
  Assert-True ($slowSnapshotQuick.ExitCode -eq 0) "Slow process snapshots consumed the runtime deadline after the process exited."
  Assert-True ($slowSnapshotQuick.QuiescencePasses -ge 2) "Slow snapshots bypassed the independent quiescence window."

  $existingEnvironmentKey = "YAP_NSIS_HELPER_EXISTING_$PID"
  $missingEnvironmentKey = "YAP_NSIS_HELPER_MISSING_$PID"
  $environmentEvidence = Join-Path $processRoot "environment.json"
  [Environment]::SetEnvironmentVariable($existingEnvironmentKey, "parent-value", "Process")
  [Environment]::SetEnvironmentVariable($missingEnvironmentKey, $null, "Process")
  try {
    $environmentScript = @"
[ordered]@{
  existing = [Environment]::GetEnvironmentVariable('$existingEnvironmentKey', 'Process')
  missing = [Environment]::GetEnvironmentVariable('$missingEnvironmentKey', 'Process')
} | ConvertTo-Json | Set-Content -LiteralPath '$environmentEvidence' -Encoding utf8
"@
    $environmentEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($environmentScript))
    $environmentChild = Start-ProcessWithEnvironment `
      -FilePath "powershell.exe" `
      -ArgumentList @("-NoProfile", "-NonInteractive", "-EncodedCommand", $environmentEncoded) `
      -Environment ([ordered]@{
        $existingEnvironmentKey = "child-existing"
        $missingEnvironmentKey = "child-missing"
      })
    $environmentChild.WaitForExit()
    Assert-True ($environmentChild.ExitCode -eq 0) "Environment inheritance child failed."
    $environmentValues = Get-Content -LiteralPath $environmentEvidence -Raw | ConvertFrom-Json
    $environmentChild.Dispose()
    Assert-True ($environmentValues.existing -ceq "child-existing") "Child missed the overridden existing environment value."
    Assert-True ($environmentValues.missing -ceq "child-missing") "Child missed the new environment value."
    Assert-True (
      [Environment]::GetEnvironmentVariable($existingEnvironmentKey, "Process") -ceq "parent-value"
    ) "Parent environment was not restored."
    Assert-True (
      $null -eq [Environment]::GetEnvironmentVariable($missingEnvironmentKey, "Process")
    ) "Previously absent parent environment value was not removed."
  } finally {
    [Environment]::SetEnvironmentVariable($existingEnvironmentKey, $null, "Process")
    [Environment]::SetEnvironmentVariable($missingEnvironmentKey, $null, "Process")
  }

  $timeoutPidPath = Join-Path $processRoot "timeout.pid"
  $timeoutOutPath = Join-Path $processRoot "timeout.out.log"
  $timeoutErrPath = Join-Path $processRoot "timeout.err.log"
  $timeoutScript = "`$PID | Set-Content -LiteralPath '$timeoutPidPath' -Encoding ascii; Start-Sleep -Seconds 6"
  $timeoutEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($timeoutScript))
  $timeoutError = $null
  $timeoutWatch = [System.Diagnostics.Stopwatch]::StartNew()
  try {
    Invoke-ProcessWithDeadline `
      -FilePath "powershell.exe" `
      -ArgumentList @("-NoProfile", "-EncodedCommand", $timeoutEncoded) `
      -TimeoutSeconds 0.5 `
      -QuiescenceTimeoutSeconds 0.2 `
      -StdoutPath $timeoutOutPath `
      -StderrPath $timeoutErrPath
  } catch {
    $timeoutError = $_.Exception.Message
  }
  $timeoutWatch.Stop()
  Assert-True ($timeoutError -match "exceeded the 0.5 second deadline") "Deadline helper did not fail on timeout."
  Assert-True ($timeoutError -match '"residualProcessIds":\[\]') "Deadline helper omitted successful cleanup evidence."
  Assert-True (Test-Path -LiteralPath $timeoutPidPath -PathType Leaf) "Timed process did not report its PID."
  $timedProcessId = [int](Get-Content -LiteralPath $timeoutPidPath -Raw)
  Assert-True ($timeoutWatch.Elapsed.TotalSeconds -lt 4.5) "Deadline cleanup waited for the timed process to exit naturally."
  Assert-True (-not (Test-ProcessAlive -ProcessId $timedProcessId)) "Timed process survived deadline cleanup."
  Assert-True ($timeoutError -match "terminatedProcessIds[^]]*$timedProcessId") "Deadline cleanup did not report terminating the timed process."
  Assert-True ($timeoutError -match '"reusedProcessIds":\[\]') "Creation-time precision misclassified the timed process as PID reuse."
  Assert-FileUnlocked -Path $timeoutOutPath -Message "Timed-process stdout was not reaped."
  Assert-FileUnlocked -Path $timeoutErrPath -Message "Timed-process stderr was not reaped."

  $snapshotFailurePidPath = Join-Path $processRoot "snapshot-failure.pid"
  $snapshotFailureOutPath = Join-Path $processRoot "snapshot-failure.out.log"
  $snapshotFailureErrPath = Join-Path $processRoot "snapshot-failure.err.log"
  $snapshotFailureScript = "`$PID | Set-Content -LiteralPath '$snapshotFailurePidPath' -Encoding ascii; Start-Sleep -Seconds 8"
  $snapshotFailureEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($snapshotFailureScript))
  $snapshotFailureError = $null
  $snapshotFailureWatch = [System.Diagnostics.Stopwatch]::StartNew()
  try {
    Invoke-ProcessWithDeadline `
      -FilePath "powershell.exe" `
      -ArgumentList @("-NoProfile", "-EncodedCommand", $snapshotFailureEncoded) `
      -TimeoutSeconds 4 `
      -QuiescenceTimeoutSeconds 0.2 `
      -SnapshotTimeoutSeconds 2 `
      -PollMilliseconds 1 `
      -SnapshotProviderScript "while (-not (Test-Path -LiteralPath '$snapshotFailurePidPath')) { Start-Sleep -Milliseconds 10 }; Start-Sleep -Seconds 30" `
      -StdoutPath $snapshotFailureOutPath `
      -StderrPath $snapshotFailureErrPath
  } catch {
    $snapshotFailureError = $_.Exception.Message
  }
  $snapshotFailureWatch.Stop()
  Assert-True ($snapshotFailureError -match "process snapshot exceeded the 2 second deadline") "Snapshot failure was not reported."
  Assert-True ($snapshotFailureWatch.Elapsed.TotalSeconds -lt 6) "Snapshot failure cleanup exceeded its bounded window."
  Assert-True (Test-Path -LiteralPath $snapshotFailurePidPath -PathType Leaf) "Snapshot-failure process did not report its PID."
  $snapshotFailureProcessId = [int](Get-Content -LiteralPath $snapshotFailurePidPath -Raw)
  Assert-True (-not (Test-ProcessAlive -ProcessId $snapshotFailureProcessId)) "Snapshot failure abandoned the launched process."
  Assert-FileUnlocked -Path $snapshotFailureOutPath -Message "Snapshot-failure stdout was not reaped."
  Assert-FileUnlocked -Path $snapshotFailureErrPath -Message "Snapshot-failure stderr was not reaped."

  $fastChildPidPath = Join-Path $processRoot "fast-parent-child.pid"
  $fastChildOutPath = Join-Path $processRoot "fast-parent-child.out.log"
  $fastChildErrPath = Join-Path $processRoot "fast-parent-child.err.log"
  $fastChildScript = @"
`$PID | Set-Content -LiteralPath '$fastChildPidPath' -Encoding ascii
`$parentId = [int](Get-CimInstance Win32_Process -Filter "ProcessId=`$PID").ParentProcessId
Stop-Process -Id `$parentId -Force
Start-Sleep -Seconds 8
"@
  $fastChildEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($fastChildScript))
  $fastParentError = $null
  $fastParentWatch = [System.Diagnostics.Stopwatch]::StartNew()
  try {
    Invoke-ProcessWithDeadline `
      -FilePath "powershell.exe" `
      -ArgumentList @("-NoProfile", "-EncodedCommand", $fastChildEncoded) `
      -TimeoutSeconds 1 `
      -QuiescenceTimeoutSeconds 0.2 `
      -SnapshotTimeoutSeconds 2 `
      -SnapshotProviderScript "while (-not (Test-Path -LiteralPath '$fastChildPidPath')) { Start-Sleep -Milliseconds 10 }; Start-Sleep -Milliseconds 300" `
      -StdoutPath $fastChildOutPath `
      -StderrPath $fastChildErrPath
  } catch {
    $fastParentError = $_.Exception.Message
  }
  $fastParentWatch.Stop()
  Assert-True (
    $fastParentError -match "exceeded the 1 second deadline"
  ) "Fast-parent descendants were not included in the runtime deadline. Received: $fastParentError"
  Assert-True ($fastParentWatch.Elapsed.TotalSeconds -lt 5) "Fast-parent cleanup exceeded its bounded window."
  Assert-True (Test-Path -LiteralPath $fastChildPidPath -PathType Leaf) "Fast-parent child did not report its PID."
  $fastChildProcessId = [int](Get-Content -LiteralPath $fastChildPidPath -Raw)
  Assert-True (-not (Test-ProcessAlive -ProcessId $fastChildProcessId)) "Fast-parent child escaped deadline cleanup."
  Assert-FileUnlocked -Path $fastChildOutPath -Message "Fast-parent stdout was not reaped."
  Assert-FileUnlocked -Path $fastChildErrPath -Message "Fast-parent stderr was not reaped."

  $childIdPath = Join-Path $processRoot "child.pid"
  $childScript = @"
`$child = Start-Process powershell.exe -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30') -PassThru
`$child.Id | Set-Content -LiteralPath '$childIdPath' -Encoding ascii
Start-Sleep -Seconds 30
"@
  $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childScript))
  $parent = Start-Process powershell.exe -ArgumentList @("-NoProfile", "-EncodedCommand", $encoded) -PassThru -WindowStyle Hidden
  $deadline = [DateTime]::UtcNow.AddSeconds(5)
  while (-not (Test-Path -LiteralPath $childIdPath) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
  }
  Assert-True (Test-Path -LiteralPath $childIdPath -PathType Leaf) "Child process did not report its PID."
  $childId = [int](Get-Content -LiteralPath $childIdPath -Raw)
  $tree = @(Get-ProcessTreeIds -RootProcessId $parent.Id)
  Assert-True ($tree -contains $childId) "Process-tree discovery omitted the child."
  $cleanup = Stop-ProcessTreeBounded -RootProcessId $parent.Id -TimeoutSeconds 5
  Assert-True ($cleanup.DiscoveredProcessIds -contains $childId) "Cleanup evidence omitted the child."
  Assert-True ($cleanup.QuiescencePasses -ge 2) "Cleanup did not wait for repeated quiescence."
  Assert-True (
    -not [string]::IsNullOrWhiteSpace(($cleanup | ConvertTo-Json -Depth 5))
  ) "Tree cleanup returned evidence that Windows PowerShell cannot serialize."
  Assert-True (-not (Test-ProcessAlive -ProcessId $parent.Id)) "Parent process survived bounded termination."
  Assert-True (-not (Test-ProcessAlive -ProcessId $childId)) "Child process survived bounded termination."
  $parent.Dispose()

  $snapshotCountPath = Join-Path $processRoot "independent-quiescence-snapshots.txt"
  Set-Content -LiteralPath $snapshotCountPath -Value "0" -Encoding ascii
  $fakeProcessId = 2147483000
  $independentQuiescenceSnapshot = @"
`$count = [int](Get-Content -LiteralPath '$snapshotCountPath' -Raw)
Set-Content -LiteralPath '$snapshotCountPath' -Value (`$count + 1) -Encoding ascii
Start-Sleep -Milliseconds 70
if (`$count -eq 0) {
  [pscustomobject]@{ ProcessId = $fakeProcessId; ParentProcessId = 0; ExecutablePath = `$null; CreationIdentity = 'synthetic-process' }
}
"@
  $independentQuiescenceWatch = [System.Diagnostics.Stopwatch]::StartNew()
  $independentQuiescence = Stop-TrackedProcessesBounded `
    -ProcessIds @($fakeProcessId) `
    -TimeoutSeconds 5 `
    -QuiescenceTimeoutSeconds 0.5 `
    -SnapshotTimeoutSeconds 2 `
    -PollMilliseconds 1 `
    -SnapshotProviderScript $independentQuiescenceSnapshot
  $independentQuiescenceWatch.Stop()
  Assert-True ($independentQuiescence.QuiescencePasses -ge 2) "Independent quiescence window did not complete."
  Assert-True ($independentQuiescenceWatch.Elapsed.TotalSeconds -ge 0.45) "Cleanup returned before the configured quiescence interval."
  Assert-True ($independentQuiescence.DiscoveredProcessIds -contains $fakeProcessId) "Synthetic tracked process disappeared from evidence."

  Assert-Throws {
    Stop-TrackedProcessesBounded `
      -ProcessIds @(2147483001) `
      -TimeoutSeconds 1 `
      -QuiescenceTimeoutSeconds 0.2 `
      -SnapshotTimeoutSeconds 0.1 `
      -PollMilliseconds 1 `
      -SnapshotProviderScript "Start-Sleep -Seconds 30"
  } "process snapshot exceeded the 0.1 second deadline"

  $reuseSnapshotCountPath = Join-Path $processRoot "pid-reuse-snapshots.txt"
  $reuseStopCountPath = Join-Path $processRoot "pid-reuse-stops.txt"
  Set-Content -LiteralPath $reuseSnapshotCountPath -Value "0" -Encoding ascii
  Set-Content -LiteralPath $reuseStopCountPath -Value "0" -Encoding ascii
  $reusedProcessId = 2147483002
  $pidReuseSnapshot = @"
`$count = [int](Get-Content -LiteralPath '$reuseSnapshotCountPath' -Raw)
Set-Content -LiteralPath '$reuseSnapshotCountPath' -Value (`$count + 1) -Encoding ascii
if (`$count -eq 0) {
  [pscustomobject]@{ ProcessId = $reusedProcessId; ParentProcessId = 0; ExecutablePath = `$null; CreationIdentity = 'first-process' }
} elseif (`$count -eq 1) {
  [pscustomobject]@{ ProcessId = $reusedProcessId; ParentProcessId = 0; ExecutablePath = `$null; CreationIdentity = 'reused-process' }
}
"@
  $reuseStopper = {
    param($Process)
    $count = [int](Get-Content -LiteralPath $reuseStopCountPath -Raw)
    Set-Content -LiteralPath $reuseStopCountPath -Value ($count + 1) -Encoding ascii
  }
  $reuseCleanup = Stop-TrackedProcessesBounded `
    -ProcessIds @($reusedProcessId) `
    -TimeoutSeconds 1 `
    -QuiescenceTimeoutSeconds 0.05 `
    -SnapshotTimeoutSeconds 1 `
    -PollMilliseconds 1 `
    -SnapshotProviderScript $pidReuseSnapshot `
    -ProcessStopper $reuseStopper
  Assert-True (
    [int](Get-Content -LiteralPath $reuseStopCountPath -Raw) -eq 1
  ) "PID reuse caused the replacement process to be terminated."
  Assert-True ($reuseCleanup.ReusedProcessIds -contains $reusedProcessId) "PID reuse was not reported in cleanup evidence."

  $deleteRoot = Get-ValidatedChildPath -Root $testRoot -Token "delete-me"
  Initialize-ValidatedTree -Root $testRoot -Candidate $deleteRoot | Out-Null
  Set-Content -LiteralPath (Join-Path $deleteRoot "evidence.txt") -Value "bounded"
  Remove-ValidatedTree -Root $testRoot -Candidate $deleteRoot
  Assert-True (-not (Test-Path -LiteralPath $deleteRoot)) "Validated recursive cleanup left its tree."

  $swapRoot = Get-ValidatedChildPath -Root $testRoot -Token "swap-before-delete"
  Initialize-ValidatedTree -Root $testRoot -Candidate $swapRoot | Out-Null
  $swapTarget = Get-ValidatedChildPath -Root $externalRoot -Token "swap-target"
  Initialize-ValidatedTree -Root $externalRoot -Candidate $swapTarget | Out-Null
  Set-Content -LiteralPath (Join-Path $swapTarget "must-survive.txt") -Value "keep"
  Assert-Throws {
    Remove-ValidatedTree -Root $testRoot -Candidate $swapRoot -BeforeQuarantineRevalidation {
      param($QuarantinePath)
      Remove-Item -LiteralPath $QuarantinePath -Recurse -Force
      New-Item -ItemType Junction -Path $QuarantinePath -Target $swapTarget | Out-Null
    }
  } "Reparse point|redirect|quarantine"
  Assert-True (Test-Path -LiteralPath (Join-Path $swapTarget "must-survive.txt")) "Quarantine swap cleanup touched the redirect target."
  $swapQuarantine = Join-Path $testRoot ".swap-before-delete.delete-quarantine"
  if (Test-Path -LiteralPath $swapQuarantine) {
    [System.IO.Directory]::Delete($swapQuarantine, $false)
  }
  Remove-ValidatedTree -Root $externalRoot -Candidate $swapTarget

  $lockRoot = Get-ValidatedChildPath -Root $testRoot -Token "profile-lock"
  New-Item -ItemType Directory -Force $lockRoot | Out-Null
  $deniedMutexFactory = { param($Name) throw [System.UnauthorizedAccessException]::new("global namespace denied") }
  $firstLock = Enter-SmokeRunLock -ProductKey "Yap.Test" -ProfileRoot $lockRoot -MutexFactory $deniedMutexFactory
  try {
    Assert-True ($firstLock.Kind -ceq "ProfileFile") "Denied Global mutex did not use the profile-lock fallback."
    Assert-Throws {
      Enter-SmokeRunLock -ProductKey "Yap.Test" -ProfileRoot $lockRoot -MutexFactory $deniedMutexFactory
    } "already owns"
  } finally {
    Exit-SmokeRunLock -Lock $firstLock
  }

  $unownedRoot = Get-ValidatedChildPath -Root $testRoot -Token "unowned"
  New-Item -ItemType Directory -Force $unownedRoot | Out-Null
  Assert-Throws {
    Remove-ValidatedTree -Root $testRoot -Candidate $unownedRoot
  } "test-data sentinel"
  Remove-Item -LiteralPath $unownedRoot -Force -ErrorAction Stop

  $productionRoot = Get-ValidatedChildPath -Root $tempRoot -Token "Yap"
  Assert-Throws {
    Initialize-ValidatedTree -Root $tempRoot -Candidate $productionRoot
  } "production Yap directory"

  Write-Output "NSIS smoke helper tests passed."
} finally {
  foreach ($candidate in @($testRoot, $externalRoot, $processRoot)) {
    if (Test-Path -LiteralPath $candidate) {
      Assert-NoProcessesUnderPath -Root $candidate
      if ($candidate -ceq $processRoot) {
        Assert-NoReparsePoints -Path $candidate
        foreach ($file in Get-ChildItem -LiteralPath $candidate -Force -File) {
          Remove-Item -LiteralPath $file.FullName -Force -ErrorAction Stop
        }
        Remove-Item -LiteralPath $candidate -Force -ErrorAction Stop
      } else {
        Remove-ValidatedTree -Root $tempRoot -Candidate $candidate
      }
    }
  }
}
