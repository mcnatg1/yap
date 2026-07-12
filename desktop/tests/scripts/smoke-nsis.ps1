param(
  [string]$BundleDirectory = "",
  [string]$ExpectedInstallerSha256 = "",
  [Parameter(Mandatory)]
  [ValidateSet("LocalSafe", "TestIdentityDelete", "IsolatedProductionDelete")]
  [string]$Mode
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "nsis-smoke-helpers.psm1") -Force

$normalizedExpectedInstallerSha256 = $ExpectedInstallerSha256.Trim().ToUpperInvariant()
if (
  -not [string]::IsNullOrWhiteSpace($normalizedExpectedInstallerSha256) -and
  $normalizedExpectedInstallerSha256 -cnotmatch '^[0-9A-F]{64}$'
) {
  throw "Expected installer SHA-256 must contain exactly 64 hexadecimal characters."
}
$expectedInstallerSha256Evidence = if (
  [string]::IsNullOrWhiteSpace($normalizedExpectedInstallerSha256)
) { $null } else { $normalizedExpectedInstallerSha256 }

$usesProductionIdentity = $Mode -eq "IsolatedProductionDelete"
$deletesAppData = $Mode -ne "LocalSafe"
$isGitHubHosted = $env:GITHUB_ACTIONS -eq "true" -and $env:RUNNER_ENVIRONMENT -eq "github-hosted"
$isWindowsSandbox = $env:USERNAME -eq "WDAGUtilityAccount"
$disposableProfileSentinel = Join-Path $env:USERPROFILE ".yap-disposable-test-profile"
$isDedicatedTestAccount = (
  $env:USERNAME -match "^Yap(?:Test|Smoke)" -and
  (Test-Path -LiteralPath $disposableProfileSentinel -PathType Leaf) -and
  (Get-Content -LiteralPath $disposableProfileSentinel -Raw).TrimEnd() -ceq "yap-disposable-profile-v1"
)
if ($usesProductionIdentity -and -not ($isGitHubHosted -or $isWindowsSandbox -or $isDedicatedTestAccount)) {
  throw "Production app-data deletion requires a GitHub-hosted runner, Windows Sandbox, or a sentinel-marked YapTest/YapSmoke disposable account."
}

$productName = if ($usesProductionIdentity) { "Yap" } else { "Yap.Test" }
$bundleId = if ($usesProductionIdentity) { "com.mcnatg1.yap" } else { "com.mcnatg1.yap.test" }
$appDataLeaf = $productName
$expectedBinaryName = if ($usesProductionIdentity) { "yap-desktop.exe" } else { "yap-test.exe" }

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
$runToken = Assert-SafePathToken -Token ([Guid]::NewGuid().ToString("N"))
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
  uninstallRegistry = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$productName"
  installRegistry = "HKCU:\Software\mcnatg1\$productName"
  startMenuShortcut = Join-Path ([Environment]::GetFolderPath("Programs")) "$productName.lnk"
  desktopShortcut = Join-Path ([Environment]::GetFolderPath("DesktopDirectory")) "$productName.lnk"
  roamingData = Join-Path $env:APPDATA $bundleId
  legacyLocalData = Join-Path $env:LOCALAPPDATA $bundleId
  yapLocalData = Join-Path $env:LOCALAPPDATA $appDataLeaf
  deleteQuarantine = Join-Path $env:LOCALAPPDATA "$productName.delete-quarantine"
}
$installFootprintNames = @("installRoot", "uninstallRegistry", "installRegistry", "startMenuShortcut", "desktopShortcut")
$dataFootprintNames = @("roamingData", "legacyLocalData", "yapLocalData")
$dataMarkerContents = "preserve-then-delete-$runToken"
$dataMarkerPaths = @{}
$destructiveSentinelName = ".yap-destructive-uninstall-test"
$destructiveSentinelContents = $runToken

Assert-PathIsNotReparsePoint -Path $tempRoot
New-Item -ItemType Directory -Force $resultsRoot | Out-Null

$events = [System.Collections.Generic.List[object]]::new()
$trackedProcessIds = [System.Collections.Generic.HashSet[int]]::new()
$evidence = [ordered]@{
  schemaVersion = 1
  mode = $Mode
  productName = $productName
  bundleId = $bundleId
  status = "running"
  phase = "initialized"
  installer = $null
  installerCandidates = @()
  artifactIntegrity = [ordered]@{
    expectedSha256 = $expectedInstallerSha256Evidence
    beforeSha256 = $null
    afterSha256 = $null
    matchedBefore = $null
    matchedAfter = $null
  }
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
    defaultUninstall = $null
    reinstall = $null
    uninstall = $null
    cleanupUninstall = $null
    cleanup = $null
  }
  uninstallFootprint = [ordered]@{
    expected = $footprintPaths
    preexisting = @()
    presentAfterInstall = @()
    residualAfterDefaultUninstall = @()
    defaultPreservedData = @()
    defaultPreservedInstallerState = @()
    explicitDeletion = [ordered]@{
      requested = $false
      residual = @()
    }
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

function Remove-OwnedDeleteQuarantine {
  param([switch]$AllowPriorRunToken)

  $quarantinePath = [System.IO.Path]::GetFullPath([string]$footprintPaths.deleteQuarantine)
  if (-not (Test-Path -LiteralPath $quarantinePath)) { return }
  $expectedPath = [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA "$productName.delete-quarantine"))
  if ($quarantinePath -cne $expectedPath) {
    throw "Delete-quarantine cleanup resolved an unexpected path: $quarantinePath"
  }
  Assert-NoReparsePoints -Path $quarantinePath
  $sentinelPath = Join-Path $quarantinePath $destructiveSentinelName
  if (-not (Test-Path -LiteralPath $sentinelPath -PathType Leaf)) {
    throw "Delete-quarantine cleanup requires its isolated-test sentinel."
  }
  $sentinelToken = (Get-Content -LiteralPath $sentinelPath -Raw).TrimEnd()
  if ($AllowPriorRunToken) {
    if ($sentinelToken -cnotmatch '^[0-9a-f]{32}$') {
      throw "Prior test delete-quarantine contains an invalid sentinel token."
    }
  } elseif ($sentinelToken -cne $runToken) {
    throw "Delete-quarantine cleanup refuses data from another test run."
  }
  Remove-ValidatedTree -Root $env:LOCALAPPDATA -Candidate $quarantinePath
}

$appProcessId = $null
$appProcessIdentity = $null
$installationStarted = $false
$ownsTestDataFootprint = -not $usesProductionIdentity
$primaryError = $null
$cleanupErrors = [System.Collections.Generic.List[Exception]]::new()
$verificationMessage = ""
$smokeLock = Enter-SmokeRunLock -ProductKey $productName -ProfileRoot $env:LOCALAPPDATA

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
  $installerProductName = $installers[0].VersionInfo.ProductName
  if ($installerProductName -cne $productName) {
    throw "NSIS smoke mode $Mode requires a $productName installer; artifact metadata reports '$installerProductName'."
  }
  $installerSha256Before = Get-Sha256Hex -Path $installer
  $evidence.artifactIntegrity.beforeSha256 = $installerSha256Before
  if (-not [string]::IsNullOrWhiteSpace($normalizedExpectedInstallerSha256)) {
    $evidence.artifactIntegrity.matchedBefore = (
      $installerSha256Before -ceq $normalizedExpectedInstallerSha256
    )
    Write-Evidence
    if (-not $evidence.artifactIntegrity.matchedBefore) {
      throw "NSIS installer SHA-256 does not match the sealed pre-smoke artifact."
    }
  }

  if (-not $usesProductionIdentity) {
    $productionPaths = @(
      [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA "Yap")),
      [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA "com.mcnatg1.yap")),
      [System.IO.Path]::GetFullPath((Join-Path $env:APPDATA "com.mcnatg1.yap"))
    )
    foreach ($dataName in $dataFootprintNames) {
      $dataPath = [System.IO.Path]::GetFullPath([string]$footprintPaths[$dataName])
      if ($productionPaths -contains $dataPath) {
        throw "Local-safe smoke resolved a production Yap data path: $dataPath"
      }
      if (Test-Path -LiteralPath $dataPath) {
        Remove-ValidatedTree `
          -Root ([System.IO.Path]::GetDirectoryName($dataPath)) `
          -Candidate $dataPath
      }
    }
    Remove-OwnedDeleteQuarantine -AllowPriorRunToken
  }

  $preexistingFootprint = @(Get-PresentFootprint)
  $evidence.uninstallFootprint.preexisting = $preexistingFootprint
  if ($preexistingFootprint.Count -gt 0) {
    throw "NSIS smoke refuses to overwrite a preexisting Yap footprint: $($preexistingFootprint -join ', ')."
  }
  Set-EvidencePhase -Phase "preparing" -Message "Creating isolated custom install root."
  if (Test-Path -LiteralPath $smokeRoot) {
    Remove-ValidatedTree -Root $tempRoot -Candidate $smokeRoot
  }
  Initialize-ValidatedTree -Root $tempRoot -Candidate $smokeRoot | Out-Null
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
  foreach ($requiredEntry in $installFootprintNames) {
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
  if ($appCandidates[0].Name -cne $expectedBinaryName) {
    throw "Installed binary identity mismatch: expected $expectedBinaryName, found $($appCandidates[0].Name)."
  }
  $appBinary = $appCandidates[0].FullName
  $installedNotice = Join-Path $installRoot "THIRD_PARTY_NOTICES.md"
  $installedProvenance = Join-Path $installRoot "THIRD_PARTY_PROVENANCE.json"
  foreach ($requiredFile in @($installedNotice, $installedProvenance)) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
      throw "The installed application is missing $([System.IO.Path]::GetFileName($requiredFile))."
    }
  }

  $noticeHash = Get-Sha256Hex -Path $installedNotice
  $provenanceHash = Get-Sha256Hex -Path $installedProvenance
  if ($noticeHash -cne (Get-Sha256Hex -Path $expectedNotice)) {
    throw "The installed third-party notice does not match the reviewed repository notice."
  }
  if ($provenanceHash -cne (Get-Sha256Hex -Path $expectedProvenance)) {
    throw "The installed provenance manifest does not match the reviewed repository manifest."
  }
  $evidence.installedBinary = $appBinary
  $evidence.noticeSha256 = $noticeHash
  $evidence.provenanceSha256 = $provenanceHash

  Set-EvidencePhase -Phase "preparing-data-policy" -Message "Preparing identity-scoped data markers before launch."
  foreach ($dataName in $dataFootprintNames) {
    $dataPath = [System.IO.Path]::GetFullPath([string]$footprintPaths[$dataName])
    $dataParent = [System.IO.Path]::GetDirectoryName($dataPath)
    if (-not (Test-StrictChildPath -Root $dataParent -Candidate $dataPath)) {
      throw "Data footprint path is not a strict child of its known parent: $dataPath"
    }
    Assert-PathIsNotReparsePoint -Path $dataParent
    if ($usesProductionIdentity) {
      New-Item -ItemType Directory -Force $dataPath | Out-Null
      Assert-NoReparsePoints -Path $dataPath
    } else {
      Initialize-ValidatedTree -Root $dataParent -Candidate $dataPath | Out-Null
    }
    $markerPath = Join-Path $dataPath "nsis-smoke-$runToken.txt"
    Set-Content -LiteralPath $markerPath -Value $dataMarkerContents -Encoding ascii
    $dataMarkerPaths[$dataName] = $markerPath
  }
  if ($deletesAppData) {
    $destructiveSentinelPath = Join-Path ([string]$footprintPaths.yapLocalData) $destructiveSentinelName
    [System.IO.File]::WriteAllText(
      $destructiveSentinelPath,
      $destructiveSentinelContents,
      [System.Text.Encoding]::ASCII
    )
  }

  Set-EvidencePhase -Phase "launching" -Message "Launching the installed app for a bounded survival probe."
  $appEnvironment = if ($usesProductionIdentity) { @{} } else {
    [ordered]@{
      YAP_APP_DATA_DIR = [string]$footprintPaths.yapLocalData
      YAP_MODELS_DIR = Join-Path ([string]$footprintPaths.yapLocalData) "models"
      YAP_LIVE_RECORDINGS_DIR = Join-Path ([string]$footprintPaths.yapLocalData) "live-recordings"
      WEBVIEW2_USER_DATA_FOLDER = Join-Path ([string]$footprintPaths.legacyLocalData) "webview2"
    }
  }
  $appProcess = Start-ProcessWithEnvironment `
    -FilePath $appBinary `
    -Environment $appEnvironment `
    -StdoutPath (Join-Path $resultsRoot "app.stdout.log") `
    -StderrPath (Join-Path $resultsRoot "app.stderr.log")
  $appProcessId = $appProcess.Id
  $appProcessIdentity = $appProcess.StartTime.ToUniversalTime().Ticks.ToString([Globalization.CultureInfo]::InvariantCulture)
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
    -RootProcessIdentity $appProcessIdentity `
    -TimeoutSeconds $cleanupTimeoutSeconds
  foreach ($processId in $appCleanup.DiscoveredProcessIds) { Add-TrackedProcess -ProcessId $processId }
  $evidence.processes.app.shutdown = $appCleanup
  $appProcess.Dispose()
  Assert-NoProcessesUnderPath -Root $installRoot
  $appProcessId = $null
  $appProcessIdentity = $null

  $uninstaller = Join-Path $installRoot "uninstall.exe"
  if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
    throw "The NSIS installation did not create uninstall.exe."
  }
  Set-EvidencePhase -Phase "default-uninstalling" -Message "Verifying that default silent uninstall preserves user data."
  $defaultUninstall = Invoke-ProcessWithDeadline `
    -FilePath $uninstaller `
    -ArgumentList @("/S") `
    -TimeoutSeconds $uninstallTimeoutSeconds `
    -StdoutPath (Join-Path $resultsRoot "default-uninstall.stdout.log") `
    -StderrPath (Join-Path $resultsRoot "default-uninstall.stderr.log")
  foreach ($processId in $defaultUninstall.ProcessIds) { Add-TrackedProcess -ProcessId $processId }
  $evidence.processes.defaultUninstall = $defaultUninstall
  if ($defaultUninstall.ExitCode -ne 0) { throw "Default NSIS uninstaller exited with code $($defaultUninstall.ExitCode)." }

  Wait-PathAbsent -Path $installRoot -TimeoutSeconds $cleanupTimeoutSeconds
  Assert-NoProcessesUnderPath -Root $installRoot
  $defaultResidual = @(Get-PresentFootprint)
  $evidence.uninstallFootprint.residualAfterDefaultUninstall = $defaultResidual
  $removedInstallFootprintNames = $installFootprintNames | Where-Object { $_ -ne "installRegistry" }
  foreach ($installName in $removedInstallFootprintNames) {
    if ($defaultResidual -contains $installName) {
      throw "Default NSIS uninstall left installation footprint entry $installName."
    }
  }
  if ($defaultResidual -notcontains "installRegistry") {
    throw "Default NSIS uninstall unexpectedly removed preserved installer state."
  }
  $evidence.uninstallFootprint.defaultPreservedInstallerState = @("installRegistry")
  foreach ($dataName in $dataFootprintNames) {
    if ($defaultResidual -notcontains $dataName) {
      throw "Default NSIS uninstall removed user data entry $dataName."
    }
    $markerPath = [string]$dataMarkerPaths[$dataName]
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
      throw "Default NSIS uninstall removed user data marker $dataName."
    }
    if ((Get-Content -LiteralPath $markerPath -Raw).TrimEnd() -cne $dataMarkerContents) {
      throw "Default NSIS uninstall changed user data marker $dataName."
    }
  }
  $evidence.uninstallFootprint.defaultPreservedData = @($dataFootprintNames)

  if (-not $deletesAppData) {
    $evidence.uninstallFootprint.residualAfterUninstall = $defaultResidual
    $evidence.uninstalled = $true
    $verificationMessage = "Local-safe install, launch, and default uninstall preserved only the Yap.Test data namespace."
  } else {
    Set-EvidencePhase -Phase "reinstalling" -Message "Reinstalling the exact artifact to verify explicit data deletion."
    $reinstall = Invoke-ProcessWithDeadline `
      -FilePath $installer `
      -ArgumentList @("/S", "/D=$installRoot") `
      -TimeoutSeconds $installTimeoutSeconds `
      -StdoutPath (Join-Path $resultsRoot "reinstall.stdout.log") `
      -StderrPath (Join-Path $resultsRoot "reinstall.stderr.log")
    foreach ($processId in $reinstall.ProcessIds) { Add-TrackedProcess -ProcessId $processId }
    $evidence.processes.reinstall = $reinstall
    if ($reinstall.ExitCode -ne 0) { throw "NSIS reinstall exited with code $($reinstall.ExitCode)." }

    $uninstaller = Join-Path $installRoot "uninstall.exe"
    if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
      throw "The NSIS reinstall did not create uninstall.exe."
    }
    $expectedDeleteTarget = [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA $productName))
    $actualDeleteTarget = [System.IO.Path]::GetFullPath([string]$footprintPaths.yapLocalData)
    if ($actualDeleteTarget -cne $expectedDeleteTarget) {
      throw "Destructive uninstall resolved an unexpected app-data target: $actualDeleteTarget"
    }
    if (-not (Test-Path -LiteralPath $destructiveSentinelPath -PathType Leaf)) {
      throw "Destructive uninstall requires its isolated-run sentinel."
    }
    if ((Get-Content -LiteralPath $destructiveSentinelPath -Raw).TrimEnd() -cne $destructiveSentinelContents) {
      throw "Destructive uninstall sentinel content does not match this run."
    }
    Assert-NoReparsePoints -Path $actualDeleteTarget
    $evidence.uninstallFootprint.explicitDeletion.requested = $true
    $deleteScope = if ($usesProductionIdentity) { "production data inside the approved disposable profile" } else { "sentinel-owned Yap.Test data" }
    Set-EvidencePhase -Phase "explicit-data-uninstalling" -Message "Verifying deletion of $deleteScope."
    $uninstall = Invoke-ProcessWithDeadline `
      -FilePath $uninstaller `
      -ArgumentList @("/S", "/DELETEAPPDATA=$runToken") `
      -TimeoutSeconds $uninstallTimeoutSeconds `
      -StdoutPath (Join-Path $resultsRoot "uninstall.stdout.log") `
      -StderrPath (Join-Path $resultsRoot "uninstall.stderr.log")
    foreach ($processId in $uninstall.ProcessIds) { Add-TrackedProcess -ProcessId $processId }
    $evidence.processes.uninstall = $uninstall
    if ($uninstall.ExitCode -ne 0) { throw "Explicit-data NSIS uninstaller exited with code $($uninstall.ExitCode)." }

    Wait-PathAbsent -Path $installRoot -TimeoutSeconds $cleanupTimeoutSeconds
    Assert-NoProcessesUnderPath -Root $installRoot
    $residualFootprint = @(Get-PresentFootprint)
    $evidence.uninstallFootprint.explicitDeletion.residual = $residualFootprint
    $evidence.uninstallFootprint.residualAfterUninstall = $residualFootprint
    if ($residualFootprint.Count -gt 0) {
      throw "NSIS uninstall left footprint entries: $($residualFootprint -join ', ')."
    }
    $evidence.uninstalled = $true
    $verificationMessage = "Default uninstall preserved data; explicit deletion removed the complete $productName footprint."
  }

  Set-EvidencePhase -Phase "verifying-artifact-integrity" -Message "Verifying that the exact sealed installer remained unchanged throughout smoke."
  $installerSha256After = Get-Sha256Hex -Path $installer
  $evidence.artifactIntegrity.afterSha256 = $installerSha256After
  if (-not [string]::IsNullOrWhiteSpace($normalizedExpectedInstallerSha256)) {
    $evidence.artifactIntegrity.matchedAfter = (
      $installerSha256After -ceq $normalizedExpectedInstallerSha256
    )
    Write-Evidence
    if (-not $evidence.artifactIntegrity.matchedAfter) {
      throw "NSIS installer SHA-256 changed after its sealed pre-smoke verification."
    }
  }
  Set-EvidencePhase -Phase "verified" -Message $verificationMessage
} catch {
  $primaryError = $_.Exception
  $evidence.errors = @($evidence.errors) + $primaryError.Message
  try {
    Set-EvidencePhase -Phase "failed" -Message $primaryError.Message
  } catch {
    $cleanupErrors.Add([Exception]::new("Failure evidence write failed: $($_.Exception.Message)", $_.Exception))
  }
} finally {
  if ($null -ne $appProcessId) {
    try {
      $appCleanup = Stop-ProcessTreeBounded `
        -RootProcessId $appProcessId `
        -RootProcessIdentity $appProcessIdentity `
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
          -ArgumentList @("/S") `
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

  try {
    Assert-NoProcessesUnderPath -Root $installRoot
  } catch {
    $cleanupErrors.Add([Exception]::new("Process-footprint verification failed: $($_.Exception.Message)", $_.Exception))
  }

  if (Test-Path -LiteralPath ([string]$footprintPaths.deleteQuarantine)) {
    try {
      Remove-OwnedDeleteQuarantine
    } catch {
      $cleanupErrors.Add([Exception]::new("Delete-quarantine cleanup failed: $($_.Exception.Message)", $_.Exception))
    }
  }

  if ($ownsTestDataFootprint) {
    foreach ($dataName in $dataFootprintNames) {
      $dataPath = [string]$footprintPaths[$dataName]
      if (-not (Test-Path -LiteralPath $dataPath)) { continue }
      try {
        Remove-ValidatedTree `
          -Root ([System.IO.Path]::GetDirectoryName($dataPath)) `
          -Candidate $dataPath
      } catch {
        $cleanupErrors.Add([Exception]::new("Data-footprint cleanup failed for $dataName`: $($_.Exception.Message)", $_.Exception))
      }
    }
    $expectedTestFootprint = [ordered]@{
      installRegistry = "HKCU:\Software\mcnatg1\Yap.Test"
      uninstallRegistry = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Yap.Test"
      startMenuShortcut = Join-Path ([Environment]::GetFolderPath("Programs")) "Yap.Test.lnk"
      desktopShortcut = Join-Path ([Environment]::GetFolderPath("DesktopDirectory")) "Yap.Test.lnk"
    }
    foreach ($entry in $expectedTestFootprint.GetEnumerator()) {
      $actualPath = [string]$footprintPaths[$entry.Key]
      if ($actualPath -cne [string]$entry.Value) {
        $cleanupErrors.Add([Exception]::new("Test footprint cleanup resolved an unexpected $($entry.Key): $actualPath"))
        continue
      }
      if (-not (Test-Path -LiteralPath $actualPath)) { continue }
      try {
        if ($entry.Key -like "*Registry") {
          Remove-Item -LiteralPath $actualPath -Recurse -Force -ErrorAction Stop
        } else {
          Remove-Item -LiteralPath $actualPath -Force -ErrorAction Stop
        }
      } catch {
        $cleanupErrors.Add([Exception]::new("Test footprint cleanup failed for $($entry.Key): $($_.Exception.Message)", $_.Exception))
      }
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

  if ($null -ne $smokeLock) {
    try {
      Exit-SmokeRunLock -Lock $smokeLock
      $smokeLock = $null
    } catch {
      $cleanupErrors.Add([Exception]::new("Smoke-run lock release failed: $($_.Exception.Message)", $_.Exception))
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
