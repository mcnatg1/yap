#requires -Version 7.4
#requires -PSEdition Core

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True {
  param(
    [Parameter(Mandatory)]
    [bool]$Condition,

    [Parameter(Mandatory)]
    [string]$Message
  )

  if (-not $Condition) {
    throw $Message
  }
}

function Get-ContainedProcessTestFailure {
  param(
    [Parameter(Mandatory)]
    [Exception]$Exception
  )

  $seen = [Collections.Generic.HashSet[object]]::new(
    [Collections.Generic.ReferenceEqualityComparer]::Instance
  )
  $current = $Exception
  for ($depth = 0; $null -ne $current -and $depth -lt 64; $depth++) {
    if (-not $seen.Add($current)) {
      break
    }
    if ($current -is [Yap.NsisSmoke.ContainedProcessException]) {
      return $current
    }
    $current = $current.InnerException
  }
  return $null
}

$root = $null
$nonce = $null
$sentinel = $null

try {
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
    $failure = $null
    try { $candidate.Launch($failingRequest) } catch { $failure = Get-ContainedProcessTestFailure $_.Exception }
    Assert-True ($failure -is [Yap.NsisSmoke.ContainedProcessException]) "Scripted failure was untyped."
    Assert-True ($failure.Stage.ToString() -ceq $case.Stage) "Scripted failure reported the wrong stage."
    Assert-True $failure.CleanupProven "Scripted failure did not prove cleanup."
    Assert-True ($scenario.OpenHandleCount -eq 0) "Scripted failure leaked a test handle."
  }

  $cleanupFailure = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
  $cleanupFailure.FailurePoint = [Yap.NsisSmoke.Testing.ScriptedFailurePoint]::ResumeThread
  $cleanupFailure.CleanupWaitSignals = $false
  $failure = $null
  try {
    [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($cleanupFailure).Launch($failingRequest)
  } catch { $failure = Get-ContainedProcessTestFailure $_.Exception }
  Assert-True (-not $failure.CleanupProven) "Failed cleanup was promoted to success."
  Assert-True ($failure.CleanupErrors.Count -gt 0) "Failed cleanup lost its evidence."
  $cleanupErrorCount = $failure.CleanupErrors.Count
  $mutationThrew = $false
  try { $failure.CleanupErrors.Add("caller mutation") } catch { $mutationThrew = $true }
  Assert-True $mutationThrew "CleanupErrors was mutable."
  Assert-True ($failure.CleanupErrors.Count -eq $cleanupErrorCount) "CleanupErrors changed after construction."

  $postCreateHandoff = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
  $postCreateHandoff.FailurePoint = [Yap.NsisSmoke.Testing.ScriptedFailurePoint]::PostCreateOwnershipHandoff
  $postCreateLease = $null
  $failure = $null
  try {
    $postCreateLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher(
      $postCreateHandoff
    ).Launch($failingRequest)
  } catch { $failure = Get-ContainedProcessTestFailure $_.Exception }
  Assert-True ($failure -is [Yap.NsisSmoke.ContainedProcessException]) "Post-create handoff failure was untyped."
  Assert-True ($failure.Stage.ToString() -ceq "CreateProcess") "Post-create handoff failure reported the wrong stage."
  Assert-True ($null -eq $postCreateLease) "Post-create handoff failure returned a lease."
  Assert-True ($postCreateHandoff.LiveChildCount -eq 0) "Post-create handoff failure left a scripted child alive."
  Assert-True ($postCreateHandoff.OpenHandleCount -eq 0) "Post-create handoff failure leaked a raw test handle."

  foreach ($releaseCase in @(
    @{ Point = "ReleaseParentStdin"; Stage = "Dispose" },
    @{ Point = "ReleaseParentStdout"; Stage = "Dispose" },
    @{ Point = "ReleaseParentStderr"; Stage = "Dispose" },
    @{ Point = "ReleaseThread"; Stage = "Resume" },
    @{ Point = "ReleaseAttributeList"; Stage = "Dispose" },
    @{ Point = "ReleaseInheritedHandleArray"; Stage = "Dispose" },
    @{ Point = "ReleaseCommandBuffer"; Stage = "Dispose" },
    @{ Point = "ReleaseEnvironmentBuffer"; Stage = "Dispose" }
  )) {
    $releaseFailure = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
    $releaseFailure.FailurePoint = [Enum]::Parse(
      [Yap.NsisSmoke.Testing.ScriptedFailurePoint],
      [string]$releaseCase.Point
    )
    $releaseLease = $null
    $failure = $null
    try {
      $releaseLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher(
        $releaseFailure
      ).Launch($failingRequest)
    } catch { $failure = Get-ContainedProcessTestFailure $_.Exception }
    Assert-True ($failure.Stage.ToString() -ceq $releaseCase.Stage) "Launch-only release reported the wrong stage."
    Assert-True (-not $failure.CleanupProven) "A launch-only release failure was promoted to proven cleanup."
    Assert-True ($failure.CleanupErrors.Count -gt 0) "Launch-only release failure lost cleanup evidence."
    Assert-True ($null -eq $releaseLease) "A lease escaped after launch-only release failed."
    Assert-True ($releaseFailure.OpenHandleCount -eq 0) "Launch-only release failure leaked a test handle."
  }

  foreach ($resumeCase in @(
    @{ Result = [uint32]0; NativeError = $null },
    @{ Result = [uint32]2; NativeError = $null },
    @{ Result = [uint32]::MaxValue; NativeError = 5 }
  )) {
    $resume = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
    $resume.ResumeThreadResult = [uint32]$resumeCase.Result
    $resume.ResumeThreadLastError = 5
    $failure = $null
    try {
      [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($resume).Launch($failingRequest)
    } catch { $failure = Get-ContainedProcessTestFailure $_.Exception }
    Assert-True ($failure.Stage.ToString() -ceq "Resume") "Unexpected suspend count reported the wrong stage."
    Assert-True ($failure.NativeErrorCode -eq $resumeCase.NativeError) "Resume failure retained an inapplicable/stale native error."
    Assert-True $failure.CleanupProven "Resume failure cleanup was not proven."
    Assert-True ($resume.ResumeThreadCallCount -eq 1) "ResumeThread was not called exactly once."
  }

  $concurrentScenarios = [Yap.NsisSmoke.Testing.ScriptedNativeScenario[]]::new(8)
  for ($index = 0; $index -lt $concurrentScenarios.Length; $index++) {
    $concurrentScenarios[$index] = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
    $concurrentScenarios[$index].ResumeThreadResult = [uint32]::MaxValue
    $concurrentScenarios[$index].ResumeThreadLastError = 1200 + $index
  }
  $concurrentErrors = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CaptureConcurrentLaunchFailures(
    $failingRequest,
    $concurrentScenarios
  )
  Assert-True ($concurrentErrors.Count -eq $concurrentScenarios.Length) "Concurrent launch failures were lost."
  for ($index = 0; $index -lt $concurrentScenarios.Length; $index++) {
    Assert-True (
      $concurrentErrors[$index].NativeErrorCode -eq $concurrentScenarios[$index].ResumeThreadLastError
    ) "Concurrent launch failure consumed another call's native error."
    Assert-True $concurrentErrors[$index].CleanupProven "Concurrent launch failure cleanup was not proven."
  }

  $identityMismatch = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
  $identityMismatch.CapturedExecutablePath = Join-Path $root "not-the-requested-image.exe"
  $identityLease = $null
  $failure = $null
  try {
    $identityLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher(
      $identityMismatch
    ).Launch($failingRequest)
  } catch { $failure = Get-ContainedProcessTestFailure $_.Exception }
  Assert-True ($failure.Stage.ToString() -ceq "CaptureIdentity") "Identity mismatch reported the wrong stage."
  Assert-True ($identityMismatch.ResumeThreadCallCount -eq 0) "Identity mismatch resumed child code."
  Assert-True $failure.CleanupProven "Identity mismatch cleanup was not proven."
  Assert-True ($null -eq $identityLease) "Identity mismatch returned a lease."
  Assert-True ($identityMismatch.OpenHandleCount -eq 0) "Identity mismatch leaked a test handle."

  foreach ($operationCase in @(
    @{ Property = "WaitForSingleObjectLastError"; Code = 2101; Stage = "Wait"; Operation = "Wait" },
    @{ Property = "GetExitCodeProcessLastError"; Code = 2102; Stage = "Wait"; Operation = "ExitCode" },
    @{ Property = "QueryInformationJobObjectLastError"; Code = 2103; Stage = "Wait"; Operation = "Query" },
    @{ Property = "TerminateJobObjectLastError"; Code = 2104; Stage = "Terminate"; Operation = "Terminate" }
  )) {
    $operationFailure = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
    $operationFailure.($operationCase.Property) = $operationCase.Code
    if ($operationCase.Operation -ceq "ExitCode") {
      $operationFailure.RootInitiallyExited = $true
    }
    $operationLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($operationFailure).Launch($failingRequest)
    $failure = $null
    try {
      switch ($operationCase.Operation) {
        "Wait" { $operationLease.WaitForRootExit([TimeSpan]::FromSeconds(1)) | Out-Null }
        "ExitCode" { $operationLease.WaitForRootExit([TimeSpan]::FromSeconds(1)) | Out-Null }
        "Query" { $operationLease.WaitForQuiescence([TimeSpan]::FromSeconds(1)) | Out-Null }
        "Terminate" { $operationLease.TerminateAndWait(0x59504150, [TimeSpan]::FromSeconds(1)) | Out-Null }
      }
    } catch { $failure = Get-ContainedProcessTestFailure $_.Exception }
    Assert-True ($failure.Stage.ToString() -ceq $operationCase.Stage) "Lease failure reported the wrong stage."
    Assert-True ($failure.NativeErrorCode -eq $operationCase.Code) "Lease failure retained the wrong native error."
    Assert-True (-not $failure.CleanupProven) "A failed lease operation manufactured cleanup proof."
    $operationLease.Dispose()
  }

  $rootTimeout = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
  $rootTimeout.RootWaitSignals = $false
  $rootTimeoutLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($rootTimeout).Launch($failingRequest)
  $rootTimeoutReport = $rootTimeoutLease.WaitForRootExit([TimeSpan]::FromMilliseconds(25))
  Assert-True (-not $rootTimeoutReport.Exited) "A root wait timeout was reported as an exit."
  Assert-True ($null -eq $rootTimeoutReport.ExitCode) "A root wait timeout manufactured an exit code."
  $rootTimeoutLease.Dispose()

  $jobTimeout = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
  $jobTimeout.JobRemainsActive = $true
  $jobTimeoutLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($jobTimeout).Launch($failingRequest)
  $failure = $null
  try { $jobTimeoutLease.WaitForQuiescence([TimeSpan]::FromMilliseconds(25)) } catch {
    $failure = Get-ContainedProcessTestFailure $_.Exception
  }
  Assert-True ($failure.Stage.ToString() -ceq "Wait") "Job quiescence timeout reported the wrong stage."
  Assert-True (-not $failure.CleanupProven) "Job quiescence timeout manufactured cleanup proof."
  $jobTimeoutLease.Dispose()

  $highExit = [Yap.NsisSmoke.Testing.ScriptedNativeScenario]::new()
  $highExit.RootInitiallyExited = $true
  $highDwordExitCode = [Convert]::ToUInt32("F0000001", 16)
  $highExit.RootExitCode = $highDwordExitCode
  $highLease = [Yap.NsisSmoke.Testing.ContainedProcessTestFactory]::CreateScriptedLauncher($highExit).Launch($failingRequest)
  $highReport = $highLease.WaitForRootExit([TimeSpan]::FromSeconds(1))
  Assert-True ($highReport.ExitCode -eq $highDwordExitCode) "A high DWORD exit code was narrowed."
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
  Assert-True ($lease.RootProcessId -gt 0) "The lease lost its retained root process ID."
  Assert-True ($lease.RootCreationFileTime -gt 0) "The lease lost its retained creation FILETIME."
  Assert-True (
    [StringComparer]::OrdinalIgnoreCase.Equals($lease.RootExecutablePath, [IO.Path]::GetFullPath($runtime))
  ) "The lease lost its canonical executable identity."
  $requiredOrder = @("CreateProcessSuspended", "AssignJob", "VerifyJobMembership", "CaptureIdentity", "ResumeThread")
  $observedOrder = @($success.OperationLog | Where-Object { $_ -in $requiredOrder })
  Assert-True (($observedOrder -join "|") -ceq ($requiredOrder -join "|")) "Success-path assignment/identity/resume order changed."
  Assert-True ($success.ResumeThreadCallCount -eq 1) "Success path did not resume exactly once."
  $operationLogCount = $success.OperationLog.Count
  $mutationThrew = $false
  try { $success.OperationLog.Add("caller mutation") } catch { $mutationThrew = $true }
  Assert-True $mutationThrew "Scripted operation log was mutable."
  Assert-True ($success.OperationLog.Count -eq $operationLogCount) "Scripted operation log changed after construction."
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

  Write-Output "Windows contained-process contracts passed."
}
finally {
  if ($null -ne $root -and [IO.Directory]::Exists($root)) {
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $actualRoot = [IO.Path]::GetFullPath($root).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $expectedRoot = [IO.Path]::GetFullPath((Join-Path $tempRoot "yap-launch-request-$nonce")).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $actualParent = [IO.Directory]::GetParent($actualRoot)
    $ownedPath =
      $null -ne $actualParent -and
      [StringComparer]::OrdinalIgnoreCase.Equals($actualRoot, $expectedRoot) -and
      [StringComparer]::OrdinalIgnoreCase.Equals($actualParent.FullName.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar), $tempRoot)

    $ownedSentinel = $false
    if ($ownedPath -and $null -ne $sentinel -and [IO.File]::Exists($sentinel)) {
      $expectedSentinel = Join-Path $actualRoot ".yap-launch-request-v1"
      if ([StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($sentinel), $expectedSentinel)) {
        $sentinelStream = [IO.File]::Open($sentinel, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
          $ownedSentinel = $sentinelStream.Length -eq 0
        }
        finally {
          $sentinelStream.Dispose()
        }
      }
    }

    if ($ownedPath -and $ownedSentinel) {
      Remove-Item -LiteralPath $actualRoot -Recurse -Force -ErrorAction Stop
    }
  }
}
