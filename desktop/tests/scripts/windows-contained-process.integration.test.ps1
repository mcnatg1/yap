#requires -Version 7.4
#requires -PSEdition Core

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$sourcePath = Join-Path $PSScriptRoot "windows-contained-process.cs"
$fixturePath = Join-Path $PSScriptRoot "windows-contained-process-fixture.ps1"
$powerShellExecutable = Join-Path $PSHOME "pwsh.exe"
if (-not ("Yap.NsisSmoke.LaunchRequest" -as [type])) {
  Add-Type -Path $sourcePath
}

function Assert-True([bool]$Condition, [string]$Message) {
  if (-not $Condition) { throw $Message }
}

function New-Request {
  param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string[]]$Arguments,
    [Parameter(Mandatory)][string]$Name,
    [string]$WorkingDirectory = $script:testRoot,
    [Collections.IDictionary]$Environment = ([ordered]@{})
  )

  return [Yap.NsisSmoke.LaunchRequest]::Create(
    $Executable,
    $Arguments,
    (Join-Path $script:testRoot "$Name.stdout.log"),
    (Join-Path $script:testRoot "$Name.stderr.log"),
    $WorkingDirectory,
    $Environment
  )
}

function Stop-LeaseIfNeeded([Yap.NsisSmoke.ContainedProcessLease]$Lease) {
  if ($null -eq $Lease) { return }
  try {
    $root = $Lease.WaitForRootExit([TimeSpan]::FromMilliseconds(50))
    if (-not $root.Exited) {
      $Lease.TerminateAndWait(0x59504150, [TimeSpan]::FromSeconds(5)) | Out-Null
    } else {
      $Lease.WaitForQuiescence([TimeSpan]::FromSeconds(5)) | Out-Null
    }
  } catch {
    # Dispose remains the no-throw kill-on-close backstop for a failed test.
  } finally {
    $Lease.Dispose()
  }
}

$launcher = [Yap.NsisSmoke.WindowsContainedProcessLauncher]::new()
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testRoot = Join-Path $tempRoot "yap-contained-process-$([Guid]::NewGuid().ToString('N'))"
[IO.Directory]::CreateDirectory($testRoot) | Out-Null

try {
  $naturalLease = $null
  try {
    $naturalLease = $launcher.Launch((New-Request `
      -Executable $env:ComSpec `
      -Arguments @("/d", "/s", "/c", "exit /b -1") `
      -Name "natural"))
    $naturalIdentity = [ordered]@{
      processId = $naturalLease.RootProcessId
      creationFileTime = $naturalLease.RootCreationFileTime
      executablePath = $naturalLease.RootExecutablePath
    }
    $naturalRoot = $naturalLease.WaitForRootExit([TimeSpan]::FromSeconds(5))
    $naturalQuiescence = $naturalLease.WaitForQuiescence([TimeSpan]::FromSeconds(5))
    Assert-True $naturalRoot.Exited "Natural-exit root did not signal."
    Assert-True ($naturalRoot.ExitCode -eq [uint32]::MaxValue) "Unsigned exit code was not preserved."
    Assert-True ($naturalIdentity.processId -gt 0) "Retained identity omitted the root process ID."
    Assert-True ($naturalIdentity.creationFileTime -gt 0) "Retained identity omitted creation time."
    Assert-True (
      [IO.Path]::GetFullPath($naturalIdentity.executablePath) -ieq [IO.Path]::GetFullPath($env:ComSpec)
    ) "Retained identity reported the wrong executable."
    Assert-True $naturalQuiescence.Quiescent "Natural-exit Job did not become quiescent."
  } finally {
    Stop-LeaseIfNeeded $naturalLease
  }

  $timeoutLease = $null
  try {
    $timeoutLease = $launcher.Launch((New-Request `
      -Executable $powerShellExecutable `
      -Arguments @("-NoLogo", "-NoProfile", "-NonInteractive", "-File", $fixturePath, "-Mode", "Sleep") `
      -Name "timeout"))
    $timeoutRoot = $timeoutLease.WaitForRootExit([TimeSpan]::FromMilliseconds(200))
    Assert-True (-not $timeoutRoot.Exited) "Runtime timeout fixture exited before its deadline."
    $termination = $timeoutLease.TerminateAndWait(0x59504150, [TimeSpan]::FromSeconds(5))
    Assert-True $termination.RootExit.Exited "Terminated root did not signal."
    Assert-True $termination.Quiescence.Quiescent "Terminated Job did not become quiescent."
    Assert-True ($termination.RequestedExitCode -eq 0x59504150) "Termination evidence lost the requested code."
  } finally {
    Stop-LeaseIfNeeded $timeoutLease
  }

  $childPidPath = Join-Path $testRoot "descendant.pid"
  $descendantLease = $null
  try {
    $descendantLease = $launcher.Launch((New-Request `
      -Executable $powerShellExecutable `
      -Arguments @(
        "-NoLogo", "-NoProfile", "-NonInteractive", "-File", $fixturePath,
        "-Mode", "Descendant", "-ChildPidPath", $childPidPath
      ) `
      -Name "descendant"))
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $childPidPath -PathType Leaf) -and $deadline.Elapsed -lt [TimeSpan]::FromSeconds(5)) {
      Start-Sleep -Milliseconds 25
    }
    Assert-True (Test-Path -LiteralPath $childPidPath -PathType Leaf) "Descendant fixture did not publish its child PID."
    $childPid = [int]([IO.File]::ReadAllText($childPidPath))
    Assert-True ($childPid -gt 0) "Descendant fixture published an invalid child PID."
    $descendantLease.TerminateAndWait(0x59504150, [TimeSpan]::FromSeconds(5)) | Out-Null
    $childAlive = $true
    try {
      $child = [Diagnostics.Process]::GetProcessById($childPid)
      $childAlive = -not $child.HasExited
      $child.Dispose()
    } catch [ArgumentException] {
      $childAlive = $false
    }
    Assert-True (-not $childAlive) "A descendant survived Job termination."
  } finally {
    Stop-LeaseIfNeeded $descendantLease
  }

  $nestedResultPath = Join-Path $testRoot "nested.result"
  $nestedLease = $null
  try {
    $nestedLease = $launcher.Launch((New-Request `
      -Executable $powerShellExecutable `
      -Arguments @(
        "-NoLogo", "-NoProfile", "-NonInteractive", "-File", $fixturePath,
        "-Mode", "Nested",
        "-ContainedProcessSource", $sourcePath,
        "-NestedResultPath", $nestedResultPath,
        "-NestedStdoutPath", (Join-Path $testRoot "nested-inner.stdout.log"),
        "-NestedStderrPath", (Join-Path $testRoot "nested-inner.stderr.log")
      ) `
      -Name "nested-outer"))
    $nestedRoot = $nestedLease.WaitForRootExit([TimeSpan]::FromSeconds(10))
    Assert-True $nestedRoot.Exited "Outer Job-contained shell did not exit."
    Assert-True ($nestedRoot.ExitCode -eq 0) "Outer Job-contained shell failed."
    $nestedLease.WaitForQuiescence([TimeSpan]::FromSeconds(5)) | Out-Null
    Assert-True (
      (Test-Path -LiteralPath $nestedResultPath -PathType Leaf) -and
      [IO.File]::ReadAllText($nestedResultPath) -ceq "nested-ok"
    ) "A shell already in an outer Job could not create and use the inner Job."
  } finally {
    Stop-LeaseIfNeeded $nestedLease
  }

  $ioWorkingDirectory = Join-Path $testRoot "working directory"
  [IO.Directory]::CreateDirectory($ioWorkingDirectory) | Out-Null
  [Environment]::SetEnvironmentVariable("YAP_CONTAINED_REMOVE", "parent-value", "Process")
  $ioLease = $null
  try {
    $ioLease = $launcher.Launch((New-Request `
      -Executable $powerShellExecutable `
      -Arguments @("-NoLogo", "-NoProfile", "-NonInteractive", "-File", $fixturePath, "-Mode", "Io") `
      -Name "io" `
      -WorkingDirectory $ioWorkingDirectory `
      -Environment ([ordered]@{
        YAP_CONTAINED_OVERRIDE = "child-value"
        YAP_CONTAINED_REMOVE = $null
      })))
    $ioRoot = $ioLease.WaitForRootExit([TimeSpan]::FromSeconds(5))
    Assert-True ($ioRoot.Exited -and $ioRoot.ExitCode -eq 0) "I/O fixture failed."
    $ioLease.WaitForQuiescence([TimeSpan]::FromSeconds(5)) | Out-Null
  } finally {
    Stop-LeaseIfNeeded $ioLease
    [Environment]::SetEnvironmentVariable("YAP_CONTAINED_REMOVE", $null, "Process")
  }
  $ioStdout = [IO.File]::ReadAllText((Join-Path $testRoot "io.stdout.log"))
  $ioStderr = [IO.File]::ReadAllText((Join-Path $testRoot "io.stderr.log"))
  Assert-True ($ioStdout -match "(?m)^fixture-stdout\r?$") "Separate stdout was not captured."
  Assert-True ($ioStderr -match "(?m)^fixture-stderr\r?$") "Separate stderr was not captured."
  Assert-True ($ioStdout -match "(?m)^override=child-value\r?$") "Environment override was not applied."
  Assert-True ($ioStdout -match "(?m)^removed=\r?$") "Environment removal was not applied."
  Assert-True ($ioStdout -match [regex]::Escape("cwd=$ioWorkingDirectory")) "Working directory was not applied."

  Write-Output "Windows contained-process integration tests passed."
} finally {
  if (Test-Path -LiteralPath $testRoot) {
    Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Stop
  }
}
