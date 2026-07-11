param(
  [string]$BundleDirectory = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\..\.."))
$desktopRoot = Join-Path $repoRoot "desktop"
if ([string]::IsNullOrWhiteSpace($BundleDirectory)) {
  $BundleDirectory = Join-Path $desktopRoot "src-tauri\target\release\bundle\nsis"
}
$BundleDirectory = [System.IO.Path]::GetFullPath($BundleDirectory)

$installers = @(Get-ChildItem -LiteralPath $BundleDirectory -Filter "*-setup.exe" -File)
if ($installers.Count -ne 1) {
  throw "Expected exactly one NSIS installer in $BundleDirectory; found $($installers.Count)."
}

$tempRoot = if ($env:RUNNER_TEMP) {
  [System.IO.Path]::GetFullPath($env:RUNNER_TEMP)
} else {
  [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
}
$runToken = if ($env:GITHUB_RUN_ID) { $env:GITHUB_RUN_ID } else { $PID.ToString() }
$smokeRoot = [System.IO.Path]::GetFullPath((Join-Path $tempRoot "yap-nsis-smoke-$runToken"))
if (-not $smokeRoot.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Refusing to use an NSIS smoke directory outside the configured temporary root."
}

$installRoot = Join-Path $smokeRoot "install"
$resultsRoot = Join-Path $desktopRoot "tests\results"
$evidencePath = Join-Path $resultsRoot "nsis-smoke.json"
$expectedNotice = Join-Path $repoRoot "THIRD_PARTY_NOTICES.md"
$appProcess = $null
$uninstallAttempted = $false
$evidence = [ordered]@{
  status = "running"
  installer = $installers[0].FullName
  installedBinary = $null
  noticeSha256 = $null
  launched = $false
  uninstalled = $false
  timestampUtc = [DateTime]::UtcNow.ToString("o")
}

New-Item -ItemType Directory -Force $resultsRoot | Out-Null

try {
  New-Item -ItemType Directory -Force $smokeRoot | Out-Null
  $install = Start-Process `
    -FilePath $installers[0].FullName `
    -ArgumentList @("/S", "/D=$installRoot") `
    -PassThru `
    -Wait `
    -WindowStyle Hidden
  if ($install.ExitCode -ne 0) {
    throw "NSIS installer exited with code $($install.ExitCode)."
  }

  $appCandidates = @(
    Get-ChildItem -LiteralPath $installRoot -Filter "*.exe" -File |
      Where-Object { $_.Name -ne "uninstall.exe" }
  )
  if ($appCandidates.Count -ne 1) {
    throw "Expected exactly one installed application executable; found $($appCandidates.Count)."
  }
  $appBinary = $appCandidates[0].FullName
  $installedNotice = Join-Path $installRoot "THIRD_PARTY_NOTICES.md"
  if (-not (Test-Path -LiteralPath $installedNotice -PathType Leaf)) {
    throw "The installed application is missing THIRD_PARTY_NOTICES.md."
  }

  $expectedHash = (Get-FileHash -LiteralPath $expectedNotice -Algorithm SHA256).Hash
  $installedHash = (Get-FileHash -LiteralPath $installedNotice -Algorithm SHA256).Hash
  if ($installedHash -ne $expectedHash) {
    throw "The installed third-party notice does not match the reviewed repository notice."
  }
  $evidence.installedBinary = $appBinary
  $evidence.noticeSha256 = $installedHash

  $appProcess = Start-Process -FilePath $appBinary -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 3
  $appProcess.Refresh()
  if ($appProcess.HasExited) {
    throw "The installed Yap executable exited during the launch smoke with code $($appProcess.ExitCode)."
  }
  $evidence.launched = $true
  Stop-Process -Id $appProcess.Id -Force
  $appProcess.WaitForExit()

  $uninstaller = Join-Path $installRoot "uninstall.exe"
  if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
    throw "The NSIS installation did not create uninstall.exe."
  }
  $uninstallAttempted = $true
  $uninstall = Start-Process `
    -FilePath $uninstaller `
    -ArgumentList @("/S") `
    -PassThru `
    -Wait `
    -WindowStyle Hidden
  if ($uninstall.ExitCode -ne 0) {
    throw "NSIS uninstaller exited with code $($uninstall.ExitCode)."
  }

  $deadline = [DateTime]::UtcNow.AddSeconds(10)
  while ((Test-Path -LiteralPath $appBinary -PathType Leaf) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 250
  }
  if ((Test-Path -LiteralPath $appBinary) -or (Test-Path -LiteralPath $installedNotice)) {
    throw "NSIS uninstall left application or notice artifacts behind."
  }

  $evidence.uninstalled = $true
  $evidence.status = "passed"
} catch {
  $evidence.status = "failed"
  $evidence.error = $_.Exception.Message
  throw
} finally {
  if ($null -ne $appProcess -and -not $appProcess.HasExited) {
    Stop-Process -Id $appProcess.Id -Force -ErrorAction SilentlyContinue
  }
  $uninstaller = Join-Path $installRoot "uninstall.exe"
  if (-not $uninstallAttempted -and (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
    Start-Process -FilePath $uninstaller -ArgumentList @("/S") -Wait -WindowStyle Hidden |
      Out-Null
  }
  $evidence | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $evidencePath -Encoding utf8
  if (Test-Path -LiteralPath $smokeRoot) {
    Remove-Item -LiteralPath $smokeRoot -Recurse -Force -ErrorAction SilentlyContinue
  }
}
