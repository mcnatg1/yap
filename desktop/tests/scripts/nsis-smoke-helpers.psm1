#requires -Version 7.4
#requires -PSEdition Core

$containedProcessSource = Join-Path $PSScriptRoot "windows-contained-process.cs"
if (-not ("Yap.NsisSmoke.LaunchRequest" -as [type])) {
  Add-Type -Path $containedProcessSource
}

$script:YapTestTreeSentinelName = ".yap-test-tree-sentinel"
$script:YapTestTreeSentinelContents = "yap-test-owned-tree-v1"

function Assert-SafePathToken {
  param([Parameter(Mandatory)][string]$Token)

  if ($Token -in @(".", "..") -or $Token -notmatch "^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$") {
    throw "Unsafe path token: $Token"
  }
  return $Token
}

function Get-PathRelativeTo {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd("\", "/")
  $candidateFull = [IO.Path]::GetFullPath($Candidate)
  $separators = [char[]]@("\", "/")
  $rootParts = @($rootFull.Split($separators, [StringSplitOptions]::RemoveEmptyEntries))
  $candidateParts = @($candidateFull.Split($separators, [StringSplitOptions]::RemoveEmptyEntries))
  $common = 0
  while (
    $common -lt $rootParts.Count -and
    $common -lt $candidateParts.Count -and
    [string]::Equals($rootParts[$common], $candidateParts[$common], [StringComparison]::OrdinalIgnoreCase)
  ) {
    $common++
  }
  if ($common -eq 0) { return $candidateFull }

  $relativeParts = [Collections.Generic.List[string]]::new()
  for ($index = $common; $index -lt $rootParts.Count; $index++) { $relativeParts.Add("..") }
  for ($index = $common; $index -lt $candidateParts.Count; $index++) {
    $relativeParts.Add($candidateParts[$index])
  }
  if ($relativeParts.Count -eq 0) { return "." }
  return [string]::Join([IO.Path]::DirectorySeparatorChar, $relativeParts)
}

function Test-StrictChildPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $relative = Get-PathRelativeTo -Root $Root -Candidate $Candidate
  if ([string]::IsNullOrWhiteSpace($relative) -or $relative -eq ".") { return $false }
  if ([IO.Path]::IsPathRooted($relative)) { return $false }
  $firstSegment = $relative.Split([IO.Path]::DirectorySeparatorChar)[0]
  return $firstSegment -ne ".."
}

function Get-ValidatedChildPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Token
  )

  $safeToken = Assert-SafePathToken -Token $Token
  $candidate = [IO.Path]::GetFullPath((Join-Path $Root $safeToken))
  if (-not (Test-StrictChildPath -Root $Root -Candidate $candidate)) {
    throw "Path token did not resolve to a strict child of the configured root."
  }
  return $candidate
}

function Get-TauriNsisToolPaths {
  param([Parameter(Mandatory)][string]$Root)

  $rootFull = [IO.Path]::GetFullPath($Root)
  if (-not (Test-Path -LiteralPath $rootFull -PathType Container)) {
    throw "Tauri NSIS cache root is missing: $rootFull"
  }
  Assert-NoReparsePoints -Path $rootFull
  $launcherPath = Join-Path $rootFull "makensis.exe"
  $compilerPath = Join-Path $rootFull "Bin\makensis.exe"
  if (-not (Test-Path -LiteralPath $launcherPath -PathType Leaf)) {
    throw "Tauri NSIS launcher is missing: $launcherPath"
  }
  if (-not (Test-Path -LiteralPath $compilerPath -PathType Leaf)) {
    throw "Tauri NSIS compiler is missing: $compilerPath"
  }
  return [pscustomobject]@{
    LauncherPath = $launcherPath
    CompilerPath = $compilerPath
  }
}

function Get-Sha256Hex {
  param([Parameter(Mandatory)][string]$Path)

  $fullPath = [IO.Path]::GetFullPath($Path)
  if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
    throw "SHA-256 input file does not exist: $fullPath"
  }
  $stream = [IO.File]::OpenRead($fullPath)
  $sha256 = [Security.Cryptography.SHA256]::Create()
  try {
    return ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "")
  } finally {
    $sha256.Dispose()
    $stream.Dispose()
  }
}

function Assert-PathIsNotReparsePoint {
  param([Parameter(Mandatory)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) { return }
  $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
  if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Reparse point is not allowed in NSIS smoke paths: $($item.FullName)"
  }
}

function Assert-NoReparsePoints {
  param([Parameter(Mandatory)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) { return }
  $pending = [Collections.Generic.Stack[string]]::new()
  $pending.Push([IO.Path]::GetFullPath($Path))
  while ($pending.Count -gt 0) {
    $current = $pending.Pop()
    $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
      throw "Reparse point is not allowed in NSIS smoke paths: $($item.FullName)"
    }
    if ($item.PSIsContainer) {
      foreach ($child in Get-ChildItem -LiteralPath $item.FullName -Force -ErrorAction Stop) {
        $pending.Push($child.FullName)
      }
    }
  }
}

function Assert-ValidatedTreeOwnership {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $rootFull = [IO.Path]::GetFullPath($Root)
  $candidateFull = [IO.Path]::GetFullPath($Candidate)
  if (-not (Test-StrictChildPath -Root $rootFull -Candidate $candidateFull)) {
    throw "Refusing recursive deletion outside a strict child of $rootFull."
  }
  $leaf = [IO.Path]::GetFileName($candidateFull.TrimEnd("\", "/"))
  if ($leaf -in @("Yap", "com.mcnatg1.yap")) {
    throw "Refusing test cleanup of the production Yap directory: $candidateFull"
  }
  if (-not (Test-Path -LiteralPath $candidateFull -PathType Container)) {
    throw "Test-owned directory does not exist: $candidateFull"
  }
  Assert-NoReparsePoints -Path $candidateFull
  $sentinel = Join-Path $candidateFull $script:YapTestTreeSentinelName
  if (-not (Test-Path -LiteralPath $sentinel -PathType Leaf)) {
    throw "Refusing recursive deletion without the test-data sentinel: $candidateFull"
  }
  if ((Get-Content -LiteralPath $sentinel -Raw).TrimEnd() -cne $script:YapTestTreeSentinelContents) {
    throw "Refusing recursive deletion with an invalid test-data sentinel: $candidateFull"
  }
}

function Initialize-ValidatedTree {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $rootFull = [IO.Path]::GetFullPath($Root)
  $candidateFull = [IO.Path]::GetFullPath($Candidate)
  if (-not (Test-StrictChildPath -Root $rootFull -Candidate $candidateFull)) {
    throw "Test-owned path must be a strict child of $rootFull."
  }
  $leaf = [IO.Path]::GetFileName($candidateFull.TrimEnd("\", "/"))
  if ($leaf -in @("Yap", "com.mcnatg1.yap")) {
    throw "Refusing to initialize the production Yap directory as test data: $candidateFull"
  }
  Assert-PathIsNotReparsePoint -Path $rootFull
  if (Test-Path -LiteralPath $candidateFull) {
    Assert-NoReparsePoints -Path $candidateFull
    $sentinel = Join-Path $candidateFull $script:YapTestTreeSentinelName
    if (Test-Path -LiteralPath $sentinel -PathType Leaf) {
      Assert-ValidatedTreeOwnership -Root $rootFull -Candidate $candidateFull
      return $candidateFull
    }
    if (@(Get-ChildItem -LiteralPath $candidateFull -Force -ErrorAction Stop).Count -gt 0) {
      throw "Refusing to claim a non-empty directory without a test-data sentinel: $candidateFull"
    }
  } else {
    New-Item -ItemType Directory -Force $candidateFull | Out-Null
  }
  Set-Content `
    -LiteralPath (Join-Path $candidateFull $script:YapTestTreeSentinelName) `
    -Value $script:YapTestTreeSentinelContents `
    -Encoding ascii
  Assert-ValidatedTreeOwnership -Root $rootFull -Candidate $candidateFull
  return $candidateFull
}

function Remove-ValidatedTree {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate,
    [scriptblock]$BeforeQuarantineRevalidation = $null
  )

  $rootFull = [IO.Path]::GetFullPath($Root)
  $candidateFull = [IO.Path]::GetFullPath($Candidate)
  if (-not (Test-Path -LiteralPath $candidateFull)) { return }
  Assert-ValidatedTreeOwnership -Root $rootFull -Candidate $candidateFull
  $leaf = [IO.Path]::GetFileName($candidateFull.TrimEnd("\", "/"))
  $quarantineFull = Join-Path ([IO.Path]::GetDirectoryName($candidateFull)) ".$leaf.delete-quarantine"
  if (-not (Test-StrictChildPath -Root $rootFull -Candidate $quarantineFull)) {
    throw "Deletion quarantine must remain a strict child of $rootFull."
  }
  if (Test-Path -LiteralPath $quarantineFull) {
    throw "Refusing recursive deletion because the fixed quarantine is not empty: $quarantineFull"
  }
  [IO.Directory]::Move($candidateFull, $quarantineFull)
  if ($null -ne $BeforeQuarantineRevalidation) {
    & $BeforeQuarantineRevalidation $quarantineFull
  }
  Assert-ValidatedTreeOwnership -Root $rootFull -Candidate $quarantineFull
  Remove-Item -LiteralPath $quarantineFull -Recurse -Force -ErrorAction Stop
  if (Test-Path -LiteralPath $quarantineFull) {
    throw "Recursive cleanup did not remove the fixed quarantine $quarantineFull."
  }
}

function Enter-SmokeRunLock {
  param(
    [Parameter(Mandatory)][string]$ProductKey,
    [Parameter(Mandatory)][string]$ProfileRoot,
    [scriptblock]$MutexFactory = $null
  )

  $safeProductKey = [regex]::Replace($ProductKey, "[^A-Za-z0-9_.-]", "_")
  if ([string]::IsNullOrWhiteSpace($safeProductKey)) { throw "Smoke-run product key is invalid." }
  $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  $mutexName = "Global\Yap.NsisSmoke.$sid.$safeProductKey"
  $mutex = $null
  try {
    $mutex = if ($null -ne $MutexFactory) {
      & $MutexFactory $mutexName
    } else {
      [System.Threading.Mutex]::new($false, $mutexName)
    }
    $owned = $false
    try {
      $owned = $mutex.WaitOne(0)
    } catch [System.Threading.AbandonedMutexException] {
      $owned = $true
    }
    if (-not $owned) {
      $mutex.Dispose()
      throw "Another $ProductKey installer smoke run already owns the isolated test namespace."
    }
    return [pscustomobject]@{ Kind = "GlobalMutex"; Handle = $mutex; Owned = $true; Name = $mutexName }
  } catch [UnauthorizedAccessException], [Security.SecurityException] {
    if ($null -ne $mutex) { $mutex.Dispose() }
  }

  $profileFull = [IO.Path]::GetFullPath($ProfileRoot)
  Assert-PathIsNotReparsePoint -Path $profileFull
  if (-not (Test-Path -LiteralPath $profileFull -PathType Container)) {
    throw "Smoke-run profile-lock root does not exist: $profileFull"
  }
  $lockPath = Join-Path $profileFull ".yap-nsis-smoke-$safeProductKey.lock"
  try {
    $stream = [IO.File]::Open(
      $lockPath,
      [IO.FileMode]::OpenOrCreate,
      [IO.FileAccess]::ReadWrite,
      [IO.FileShare]::None
    )
    return [pscustomobject]@{ Kind = "ProfileFile"; Handle = $stream; Owned = $true; Name = $lockPath }
  } catch [IO.IOException] {
    throw "Another $ProductKey installer smoke run already owns the isolated test namespace."
  }
}

function Exit-SmokeRunLock {
  param([Parameter(Mandatory)][object]$Lock)

  if ($Lock.Kind -ceq "GlobalMutex" -and $Lock.Owned) {
    $Lock.Handle.ReleaseMutex()
    $Lock.Owned = $false
  }
  $Lock.Handle.Dispose()
}

function Assert-InstallerCleanupAuthorized {
  param(
    [Parameter(Mandatory)][bool]$Authorized,
    [Parameter(Mandatory)][string]$Operation
  )

  if (-not $Authorized) {
    throw "Installer-owned mutation '$Operation' is blocked because process cleanup was not proven."
  }
}

function Invoke-YapAuthorizedCleanupMutation {
  param(
    [Parameter(Mandatory)][bool]$CleanupAuthorized,
    [Parameter(Mandatory)][string]$Name,
    [string[]]$RetainedPaths = @(),
    [Parameter(Mandatory)][scriptblock]$Action
  )

  if (-not $CleanupAuthorized) {
    return [pscustomobject]@{
      Executed = $false
      Name = $Name
      RetainedPaths = @($RetainedPaths)
    }
  }
  & $Action | Out-Null
  return [pscustomobject]@{
    Executed = $true
    Name = $Name
    RetainedPaths = @()
  }
}

function ConvertTo-ChildEnvironment {
  param([Parameter(Mandatory)][Collections.IDictionary]$Environment)

  $childEnvironment = [System.Collections.Hashtable]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($entry in $Environment.GetEnumerator()) {
    $name = $entry.Key.PSObject.BaseObject
    if ($name -isnot [string]) {
      throw "Environment names must be strings."
    }
    $value = if ($null -eq $entry.Value) {
      $null
    } else {
      $entry.Value.PSObject.BaseObject
    }
    if ($null -ne $value -and $value -isnot [string]) {
      throw "Environment values must be strings or null."
    }
    try {
      $childEnvironment.Add([string]$name, $value)
    } catch [ArgumentException] {
      throw "Environment names must be unique ignoring case."
    }
  }
  return $childEnvironment
}

function Invoke-ContainedProcess {
  param(
    [Parameter(Mandatory)][string]$FilePath,
    [string[]]$ArgumentList = @(),
    [Parameter(Mandatory)][double]$TimeoutSeconds,
    [double]$CleanupTimeoutSeconds = 10,
    [Parameter(Mandatory)][string]$StdoutPath,
    [Parameter(Mandatory)][string]$StderrPath,
    [string]$WorkingDirectory = $null,
    [Collections.IDictionary]$Environment = ([ordered]@{}),
    [string]$NsisInstallDirectory = $null
  )

  if ($TimeoutSeconds -le 0) { throw "Process timeout must be positive." }
  if ($CleanupTimeoutSeconds -le 0) { throw "Process cleanup timeout must be positive." }
  $childEnvironment = ConvertTo-ChildEnvironment -Environment $Environment

  $request = if ($PSBoundParameters.ContainsKey("NsisInstallDirectory")) {
    $typedDirectory = [Yap.NsisSmoke.NsisInstallDirectory]::Create($NsisInstallDirectory)
    [Yap.NsisSmoke.LaunchRequest]::CreateNsisInstaller(
      $FilePath,
      $ArgumentList,
      $typedDirectory,
      $StdoutPath,
      $StderrPath,
      $WorkingDirectory,
      $childEnvironment
    )
  } else {
    [Yap.NsisSmoke.LaunchRequest]::Create(
      $FilePath,
      $ArgumentList,
      $StdoutPath,
      $StderrPath,
      $WorkingDirectory,
      $childEnvironment
    )
  }

  $lease = [Yap.NsisSmoke.WindowsContainedProcessLauncher]::new().Launch($request)
  try {
    $identity = [ordered]@{
      RootProcessId = $lease.RootProcessId
      RootCreationFileTime = $lease.RootCreationFileTime
      RootExecutablePath = $lease.RootExecutablePath
    }
    $root = $lease.WaitForRootExit([TimeSpan]::FromSeconds($TimeoutSeconds))
    $timedOut = -not $root.Exited
    $termination = $null
    if ($timedOut) {
      $termination = $lease.TerminateAndWait(0x59504150, [TimeSpan]::FromSeconds($CleanupTimeoutSeconds))
      $root = $termination.RootExit
      $quiescence = $termination.Quiescence
    } else {
      $quiescence = $lease.WaitForQuiescence([TimeSpan]::FromSeconds($CleanupTimeoutSeconds))
    }

    return [pscustomobject]@{
      RootProcessId = $identity.RootProcessId
      RootCreationFileTime = $identity.RootCreationFileTime
      RootExecutablePath = $identity.RootExecutablePath
      ExitCode = $root.ExitCode
      TimedOut = $timedOut
      CleanupProven = [bool]($root.Exited -and $quiescence.Quiescent)
      RootElapsedMilliseconds = $root.ElapsedMilliseconds
      QuiescenceElapsedMilliseconds = $quiescence.ElapsedMilliseconds
      QuiescencePollIterations = $quiescence.PollIterations
      TerminationRequested = $null -ne $termination
    }
  } finally {
    $lease.Dispose()
  }
}

function Wait-PathAbsent {
  param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][double]$TimeoutSeconds
  )

  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  while ((Test-Path -LiteralPath $Path) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 100
  }
  if (Test-Path -LiteralPath $Path) {
    throw "Path remained after the $TimeoutSeconds second deadline: $Path"
  }
}

function Invoke-BoundedPathSnapshot {
  param([Parameter(Mandatory)][double]$TimeoutSeconds)

  if ($TimeoutSeconds -le 0) { throw "Process snapshot timeout must be positive." }
  $operationTimeoutSeconds = [Math]::Max(1, [int][Math]::Ceiling($TimeoutSeconds))
  $pipeline = [Management.Automation.PowerShell]::Create()
  $asyncResult = $null
  try {
    [void]$pipeline.AddScript(@"
Get-CimInstance Win32_Process -OperationTimeoutSec $operationTimeoutSeconds -ErrorAction Stop | ForEach-Object {
  [pscustomobject]@{ ProcessId = [int]`$_.ProcessId; ExecutablePath = `$_.ExecutablePath }
}
"@)
    $asyncResult = $pipeline.BeginInvoke()
    $timeoutMilliseconds = [Math]::Max(1, [int][Math]::Ceiling($TimeoutSeconds * 1000))
    if (-not $asyncResult.AsyncWaitHandle.WaitOne($timeoutMilliseconds)) {
      $pipeline.Stop()
      throw "Post-disposal process-path diagnostic exceeded the $TimeoutSeconds second deadline."
    }
    $result = @($pipeline.EndInvoke($asyncResult))
    if ($pipeline.HadErrors) {
      $message = @($pipeline.Streams.Error | ForEach-Object { $_.ToString() }) -join "; "
      throw "Post-disposal process-path diagnostic failed: $message"
    }
    return $result
  } finally {
    if ($null -ne $asyncResult) { $asyncResult.AsyncWaitHandle.Dispose() }
    $pipeline.Dispose()
  }
}

function Get-ProcessesUnderPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [double]$SnapshotTimeoutSeconds = 10
  )

  return @(
    Invoke-BoundedPathSnapshot -TimeoutSeconds $SnapshotTimeoutSeconds |
      Where-Object {
        -not [string]::IsNullOrWhiteSpace($_.ExecutablePath) -and
        (Test-StrictChildPath -Root $Root -Candidate $_.ExecutablePath)
      } |
      ForEach-Object {
        [pscustomobject]@{
          ProcessId = [int]$_.ProcessId
          ExecutablePath = $_.ExecutablePath
        }
      }
  )
}

function Assert-NoProcessesUnderPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [double]$SnapshotTimeoutSeconds = 10
  )

  $matches = @(Get-ProcessesUnderPath -Root $Root -SnapshotTimeoutSeconds $SnapshotTimeoutSeconds)
  if ($matches.Count -gt 0) {
    $footprint = $matches | ForEach-Object { "$($_.ProcessId):$($_.ExecutablePath)" }
    throw "Processes remain under the install root: $($footprint -join ', ')."
  }
}

Export-ModuleMember -Function `
  Assert-InstallerCleanupAuthorized, `
  Assert-NoProcessesUnderPath, `
  Assert-NoReparsePoints, `
  Assert-PathIsNotReparsePoint, `
  Assert-SafePathToken, `
  Enter-SmokeRunLock, `
  Exit-SmokeRunLock, `
  Get-ProcessesUnderPath, `
  Get-Sha256Hex, `
  Get-TauriNsisToolPaths, `
  Get-ValidatedChildPath, `
  Initialize-ValidatedTree, `
  Invoke-ContainedProcess, `
  Invoke-YapAuthorizedCleanupMutation, `
  Remove-ValidatedTree, `
  Test-StrictChildPath, `
  Wait-PathAbsent
