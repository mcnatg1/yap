param([string]$BundleDirectory = "")

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "nsis-smoke-helpers.psm1") -Force

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\..\.."))
$desktopRoot = Join-Path $repoRoot "desktop"
if ([string]::IsNullOrWhiteSpace($BundleDirectory)) {
  $BundleDirectory = Join-Path $desktopRoot "src-tauri\target\release\bundle\nsis"
}
$BundleDirectory = [System.IO.Path]::GetFullPath($BundleDirectory)

$tempRoot = if ($env:RUNNER_TEMP) {
  [System.IO.Path]::GetFullPath($env:RUNNER_TEMP)
} else {
  [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
}
$runToken = Assert-SafePathToken -Token $(if ($env:GITHUB_RUN_ID) { $env:GITHUB_RUN_ID } else { $PID.ToString() })
$smokeRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-smoke-$runToken"
$installRoot = Get-ValidatedChildPath -Root $smokeRoot -Token "install"
$resultsBase = Join-Path $desktopRoot "tests\results\nsis-smoke"
$resultsRoot = Get-ValidatedChildPath -Root $resultsBase -Token $runToken
$evidencePath = Join-Path $resultsRoot "evidence.json"
$activityLog = Join-Path $resultsRoot "activity.log"
$expectedNotice = Join-Path $repoRoot "THIRD_PARTY_NOTICES.md"
$expectedProvenance = Join-Path $repoRoot "THIRD_PARTY_PROVENANCE.json"
$installTimeoutSeconds = 120
$launchProbeSeconds = 3
$uninstallTimeoutSeconds = 60
$cleanupTimeoutSeconds = 10
$footprintPaths = [ordered]@{
  installRoot = $installRoot
  uninstallRegistry = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Yap"
  installRegistry = "HKCU:\Software\mcnatg1\Yap"
  startMenuShortcut = Join-Path ([Environment]::GetFolderPath("Programs")) "Yap.lnk"
  desktopShortcut = Join-Path ([Environment]::GetFolderPath("DesktopDirectory")) "Yap.lnk"
  roamingData = Join-Path $env:APPDATA "com.mcnatg1.yap"
  localData = Join-Path $env:LOCALAPPDATA "com.mcnatg1.yap"
}

Assert-PathIsNotReparsePoint -Path $tempRoot
New-Item -ItemType Directory -Force $resultsRoot | Out-Null

$events = [System.Collections.Generic.List[object]]::new()
$trackedProcessIds = [System.Collections.Generic.HashSet[int]]::new()
$evidence = [ordered]@{
  schemaVersion = 1
  status = "running"
  phase = "initialized"
  installer = $null
  installerCandidates = @()
  installRoot = $installRoot
  installedBinary = $null
  noticeSha256 = $null
  provenanceSha256 = $null
  launched = $false
  uninstalled = $false
  deadlinesSeconds = [ordered]@{
    install = $installTimeoutSeconds
    launchProbe = $launchProbeSeconds
    uninstall = $uninstallTimeoutSeconds
    cleanup = $cleanupTimeoutSeconds
  }
  processes = [ordered]@{
    install = $null
    app = $null
    uninstall = $null
    cleanupUninstall = $null
    cleanup = $null
  }
  uninstallFootprint = [ordered]@{
    expected = $footprintPaths
    preexisting = @()
    presentAfterInstall = @()
    residualAfterUninstall = @()
  }
  trackedProcessIds = @()
  events = $events
  errors = @()
  startedAtUtc = [DateTime]::UtcNow.ToString("o")
  finishedAtUtc = $null
}

function Write-Evidence {
  $temporaryPath = "$evidencePath.tmp"
  $evidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $temporaryPath -Encoding utf8
  Move-Item -LiteralPath $temporaryPath -Destination $evidencePath -Force
}

function Set-EvidencePhase([string]$Phase, [string]$Message) {
  $evidence.phase = $Phase
  $event = [ordered]@{
    atUtc = [DateTime]::UtcNow.ToString("o")
    phase = $Phase
    message = $Message
  }
  $events.Add($event)
  "[$($event.atUtc)] [$Phase] $Message" | Add-Content -LiteralPath $activityLog -Encoding utf8
  Write-Evidence
}

function Add-TrackedProcess([int]$ProcessId) {
  if ($ProcessId -gt 0) { [void]$trackedProcessIds.Add($ProcessId) }
}

function Get-PresentFootprint {
  $present = [System.Collections.Generic.List[string]]::new()
  foreach ($entry in $footprintPaths.GetEnumerator()) {
    if (Test-Path -LiteralPath $entry.Value) { $present.Add($entry.Key) }
  }
  return @($present)
}

$appProcessId = $null
$installationStarted = $false
$primaryError = $null
$cleanupErrors = [System.Collections.Generic.List[Exception]]::new()

try {
  Write-Evidence
  Set-EvidencePhase -Phase "discovering" -Message "Discovering exactly one NSIS installer."
  if (-not (Test-Path -LiteralPath $BundleDirectory -PathType Container)) {
    throw "NSIS bundle directory does not exist: $BundleDirectory"
  }
  $installers = @(Get-ChildItem -LiteralPath $BundleDirectory -Filter "*-setup.exe" -File -ErrorAction Stop)
  $evidence.installerCandidates = @($installers | ForEach-Object { $_.FullName })
  if ($installers.Count -ne 1) {
    throw "Expected exactly one NSIS installer in $BundleDirectory; found $($installers.Count)."
  }
  $installer = $installers[0].FullName
  $evidence.installer = $installer

  $preexistingFootprint = @(Get-PresentFootprint)
  $evidence.uninstallFootprint.preexisting = $preexistingFootprint
  if ($preexistingFootprint.Count -gt 0) {
    throw "NSIS smoke refuses to overwrite a preexisting Yap footprint: $($preexistingFootprint -join ', ')."
  }

  Set-EvidencePhase -Phase "preparing" -Message "Creating isolated custom install root."
  if (Test-Path -LiteralPath $smokeRoot) {
    Remove-ValidatedTree -Root $tempRoot -Candidate $smokeRoot
  }
  New-Item -ItemType Directory -Force $smokeRoot | Out-Null
  Assert-NoReparsePoints -Path $smokeRoot

  Set-EvidencePhase -Phase "installing" -Message "Starting silent NSIS installation with a bounded deadline."
  $installationStarted = $true
  $install = Invoke-ProcessWithDeadline `
    -FilePath $installer `
    -ArgumentList @("/S", "/D=$installRoot") `
    -TimeoutSeconds $installTimeoutSeconds `
    -StdoutPath (Join-Path $resultsRoot "install.stdout.log") `
    -StderrPath (Join-Path $resultsRoot "install.stderr.log")
  foreach ($processId in $install.ProcessIds) { Add-TrackedProcess -ProcessId $processId }
  $evidence.processes.install = $install
  if ($install.ExitCode -ne 0) { throw "NSIS installer exited with code $($install.ExitCode)." }
  Assert-NoReparsePoints -Path $smokeRoot
  $presentAfterInstall = @(Get-PresentFootprint)
  $evidence.uninstallFootprint.presentAfterInstall = $presentAfterInstall
  foreach ($requiredEntry in @("installRoot", "uninstallRegistry", "installRegistry", "startMenuShortcut", "desktopShortcut")) {
    if ($presentAfterInstall -notcontains $requiredEntry) {
      throw "NSIS install footprint is missing $requiredEntry."
    }
  }

  $appCandidates = @(
    Get-ChildItem -LiteralPath $installRoot -Filter "*.exe" -File -ErrorAction Stop |
      Where-Object { $_.Name -ne "uninstall.exe" }
  )
  if ($appCandidates.Count -ne 1) {
    throw "Expected exactly one installed application executable; found $($appCandidates.Count)."
  }
  $appBinary = $appCandidates[0].FullName
  $installedNotice = Join-Path $installRoot "THIRD_PARTY_NOTICES.md"
  $installedProvenance = Join-Path $installRoot "THIRD_PARTY_PROVENANCE.json"
  foreach ($requiredFile in @($installedNotice, $installedProvenance)) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
      throw "The installed application is missing $([System.IO.Path]::GetFileName($requiredFile))."
    }
  }

  $noticeHash = (Get-FileHash -LiteralPath $installedNotice -Algorithm SHA256).Hash
  $provenanceHash = (Get-FileHash -LiteralPath $installedProvenance -Algorithm SHA256).Hash
  if ($noticeHash -ne (Get-FileHash -LiteralPath $expectedNotice -Algorithm SHA256).Hash) {
    throw "The installed third-party notice does not match the reviewed repository notice."
  }
  if ($provenanceHash -ne (Get-FileHash -LiteralPath $expectedProvenance -Algorithm SHA256).Hash) {
    throw "The installed provenance manifest does not match the reviewed repository manifest."
  }
  $evidence.installedBinary = $appBinary
  $evidence.noticeSha256 = $noticeHash
  $evidence.provenanceSha256 = $provenanceHash

  Set-EvidencePhase -Phase "launching" -Message "Launching the installed app for a bounded survival probe."
  $appProcess = Start-Process `
    -FilePath $appBinary `
    -PassThru `
    -RedirectStandardOutput (Join-Path $resultsRoot "app.stdout.log") `
    -RedirectStandardError (Join-Path $resultsRoot "app.stderr.log") `
    -WindowStyle Hidden
  $appProcessId = $appProcess.Id
  Start-Sleep -Milliseconds 250
  foreach ($processId in Get-ProcessTreeIds -RootProcessId $appProcessId) {
    Add-TrackedProcess -ProcessId $processId
  }
  $evidence.processes.app = [ordered]@{
    processId = $appProcessId
    treeProcessIds = @($trackedProcessIds)
    survivalProbeSeconds = $launchProbeSeconds
  }
  Assert-ProcessSurvives -ProcessId $appProcessId -DurationSeconds $launchProbeSeconds
  $evidence.launched = $true
  $appCleanup = Stop-ProcessTreeBounded `
    -RootProcessId $appProcessId `
    -SeedProcessIds @($trackedProcessIds) `
    -TimeoutSeconds $cleanupTimeoutSeconds
  foreach ($processId in $appCleanup.DiscoveredProcessIds) { Add-TrackedProcess -ProcessId $processId }
  $evidence.processes.app.shutdown = $appCleanup
  foreach ($processId in $trackedProcessIds) {
    if (Test-ProcessAlive -ProcessId $processId) { throw "Tracked process $processId survived app shutdown." }
  }
  $appProcessId = $null

  $uninstaller = Join-Path $installRoot "uninstall.exe"
  if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
    throw "The NSIS installation did not create uninstall.exe."
  }
  Set-EvidencePhase -Phase "uninstalling" -Message "Starting silent NSIS uninstall with a bounded deadline."
  $uninstall = Invoke-ProcessWithDeadline `
    -FilePath $uninstaller `
    -ArgumentList @("/S", "/DELETEAPPDATA") `
    -TimeoutSeconds $uninstallTimeoutSeconds `
    -StdoutPath (Join-Path $resultsRoot "uninstall.stdout.log") `
    -StderrPath (Join-Path $resultsRoot "uninstall.stderr.log")
  foreach ($processId in $uninstall.ProcessIds) { Add-TrackedProcess -ProcessId $processId }
  $evidence.processes.uninstall = $uninstall
  if ($uninstall.ExitCode -ne 0) { throw "NSIS uninstaller exited with code $($uninstall.ExitCode)." }

  Wait-PathAbsent -Path $installRoot -TimeoutSeconds $cleanupTimeoutSeconds
  Assert-NoProcessesUnderPath -Root $installRoot
  foreach ($processId in $trackedProcessIds) {
    if (Test-ProcessAlive -ProcessId $processId) { throw "Tracked process $processId survived uninstall." }
  }
  $residualFootprint = @(Get-PresentFootprint)
  $evidence.uninstallFootprint.residualAfterUninstall = $residualFootprint
  if ($residualFootprint.Count -gt 0) {
    throw "NSIS uninstall left footprint entries: $($residualFootprint -join ', ')."
  }
  $evidence.uninstalled = $true
  Set-EvidencePhase -Phase "verified" -Message "Install directory, registry, shortcuts, app data, and process footprint are absent."
} catch {
  $primaryError = $_.Exception
  $evidence.errors = @($evidence.errors) + $primaryError.Message
  try {
    Set-EvidencePhase -Phase "failed" -Message $primaryError.Message
  } catch {
    $cleanupErrors.Add([Exception]::new("Failure evidence write failed: $($_.Exception.Message)", $_.Exception))
  }
} finally {
  if ($null -ne $appProcessId -and (Test-ProcessAlive -ProcessId $appProcessId)) {
    try {
      $appCleanup = Stop-ProcessTreeBounded `
        -RootProcessId $appProcessId `
        -SeedProcessIds @($trackedProcessIds) `
        -TimeoutSeconds $cleanupTimeoutSeconds
      foreach ($processId in $appCleanup.DiscoveredProcessIds) { Add-TrackedProcess -ProcessId $processId }
      $evidence.processes.cleanup = $appCleanup
    } catch {
      $cleanupErrors.Add([Exception]::new("App process cleanup failed: $($_.Exception.Message)", $_.Exception))
    }
  }

  if (Test-Path -LiteralPath $installRoot) {
    try {
      Assert-NoReparsePoints -Path $installRoot
      $uninstaller = Join-Path $installRoot "uninstall.exe"
      if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
        $cleanupUninstall = Invoke-ProcessWithDeadline `
          -FilePath $uninstaller `
          -ArgumentList @("/S", "/DELETEAPPDATA") `
          -TimeoutSeconds $uninstallTimeoutSeconds `
          -StdoutPath (Join-Path $resultsRoot "cleanup-uninstall.stdout.log") `
          -StderrPath (Join-Path $resultsRoot "cleanup-uninstall.stderr.log")
        foreach ($processId in $cleanupUninstall.ProcessIds) { Add-TrackedProcess -ProcessId $processId }
        $evidence.processes.cleanupUninstall = $cleanupUninstall
        if ($cleanupUninstall.ExitCode -ne 0) {
          throw "Cleanup uninstaller exited with code $($cleanupUninstall.ExitCode)."
        }
        Wait-PathAbsent -Path $installRoot -TimeoutSeconds $cleanupTimeoutSeconds
      } else {
        throw "Cleanup could not find the installed uninstaller."
      }
    } catch {
      $cleanupErrors.Add([Exception]::new("NSIS cleanup failed: $($_.Exception.Message)", $_.Exception))
    }
  }

  if ($trackedProcessIds.Count -gt 0) {
    try {
      $processCleanup = Stop-TrackedProcessesBounded `
        -ProcessIds @($trackedProcessIds) `
        -TimeoutSeconds $cleanupTimeoutSeconds
      foreach ($processId in $processCleanup.DiscoveredProcessIds) { Add-TrackedProcess -ProcessId $processId }
      $evidence.processes.cleanup = $processCleanup
      Assert-NoProcessesUnderPath -Root $installRoot
    } catch {
      $cleanupErrors.Add([Exception]::new("Process-footprint cleanup failed: $($_.Exception.Message)", $_.Exception))
    }
  }

  if ($installationStarted) {
    $residualFootprint = @(Get-PresentFootprint)
    $evidence.uninstallFootprint.residualAfterUninstall = $residualFootprint
    if ($residualFootprint.Count -gt 0) {
      $cleanupErrors.Add([Exception]::new(
        "Uninstall footprint remains after cleanup: $($residualFootprint -join ', ')."
      ))
    }
  }

  if (Test-Path -LiteralPath $smokeRoot) {
    try {
      Remove-ValidatedTree -Root $tempRoot -Candidate $smokeRoot
    } catch {
      $cleanupErrors.Add([Exception]::new("Temporary-tree cleanup failed: $($_.Exception.Message)", $_.Exception))
    }
  }

  foreach ($cleanupError in $cleanupErrors) {
    $evidence.errors = @($evidence.errors) + $cleanupError.Message
  }
  $evidence.finishedAtUtc = [DateTime]::UtcNow.ToString("o")
  $evidence.trackedProcessIds = @($trackedProcessIds | Sort-Object)
  $evidence.status = if ($null -eq $primaryError -and $cleanupErrors.Count -eq 0) { "passed" } else { "failed" }
  $evidence.phase = "finished"
  try {
    Write-Evidence
  } catch {
    $cleanupErrors.Add([Exception]::new("Final evidence write failed: $($_.Exception.Message)", $_.Exception))
  }
}

$allErrors = [System.Collections.Generic.List[Exception]]::new()
if ($null -ne $primaryError) { $allErrors.Add($primaryError) }
foreach ($cleanupError in $cleanupErrors) { $allErrors.Add($cleanupError) }
if ($allErrors.Count -gt 0) {
  throw [AggregateException]::new("NSIS bundle smoke failed.", $allErrors.ToArray())
}
