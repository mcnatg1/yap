#requires -Version 7.4
#requires -PSEdition Core

param(
  [string]$BundleDirectory = "",
  [string]$ExpectedInstallerSha256 = "",
  [ValidateRange(1, 900)]
  [int]$ProcessTimeoutSeconds = 180,
  [ValidateRange(1, 120)]
  [int]$LaunchTimeoutSeconds = 30
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not $IsWindows) {
  throw "The NSIS lifecycle smoke requires Windows."
}

$isHostedDisposable =
  $env:GITHUB_ACTIONS -ceq "true" -and
  $env:RUNNER_ENVIRONMENT -ceq "github-hosted"
$isExplicitDisposable = $env:YAP_DISPOSABLE_WINDOWS -ceq "1"
if (-not ($isHostedDisposable -or $isExplicitDisposable)) {
  throw "The stock NSIS lifecycle smoke may run only on a GitHub-hosted runner or an explicitly disposable Windows environment (YAP_DISPOSABLE_WINDOWS=1)."
}

$desktopRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $desktopRoot ".."))
if ([string]::IsNullOrWhiteSpace($BundleDirectory)) {
  $BundleDirectory = Join-Path $desktopRoot "src-tauri\target\release\bundle\nsis"
}
$BundleDirectory = [System.IO.Path]::GetFullPath($BundleDirectory)

$installers = @(Get-ChildItem -LiteralPath $BundleDirectory -Filter "*-setup.exe" -File)
if ($installers.Count -ne 1) {
  throw "Expected exactly one NSIS installer in $BundleDirectory; found $($installers.Count)."
}
$installer = $installers[0]
$beforeSha256 = (Get-FileHash -LiteralPath $installer.FullName -Algorithm SHA256).Hash
$normalizedExpectedSha256 = $ExpectedInstallerSha256.Trim().ToUpperInvariant()
if ($normalizedExpectedSha256) {
  if ($normalizedExpectedSha256 -notmatch "^[0-9A-F]{64}$") {
    throw "ExpectedInstallerSha256 must be a 64-character hexadecimal SHA-256 value."
  }
  if ($beforeSha256 -cne $normalizedExpectedSha256) {
    throw "The installer SHA-256 does not match the expected artifact."
  }
}

$appDataRoot = Join-Path `
  ([Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)) `
  "com.mcnatg1.yap"
$expectedInstallLocation = Join-Path `
  ([Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)) `
  "Yap"
$productRegistryPath = "HKCU:\Software\mcnatg1\Yap"
$uninstallRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Yap"
foreach ($path in @(
    $appDataRoot,
    $expectedInstallLocation,
    $productRegistryPath,
    $uninstallRegistryPath
  )) {
  if (Test-Path -LiteralPath $path) {
    throw "The disposable Windows environment is not clean: $path already exists."
  }
}

function Invoke-BoundedProcess {
  param(
    [Parameter(Mandatory)]
    [string]$FilePath,
    [Parameter(Mandatory)]
    [string[]]$ArgumentList,
    [Parameter(Mandatory)]
    [string]$Label
  )

  $process = Start-Process `
    -FilePath $FilePath `
    -ArgumentList $ArgumentList `
    -NoNewWindow `
    -PassThru
  try {
    $timeoutMilliseconds = [int][TimeSpan]::FromSeconds($ProcessTimeoutSeconds).TotalMilliseconds
    if (-not $process.WaitForExit($timeoutMilliseconds)) {
      $process.Kill($true)
      if (-not $process.WaitForExit(10000)) {
        throw "$Label timed out and its process tree could not be reaped."
      }
      throw "$Label exceeded the $ProcessTimeoutSeconds second deadline."
    }
    if ($process.ExitCode -ne 0) {
      throw "$Label failed with exit code $($process.ExitCode)."
    }
  } finally {
    if (-not $process.HasExited) {
      $process.Kill($true)
      $null = $process.WaitForExit(10000)
    }
    $process.Dispose()
  }
}

Invoke-BoundedProcess -FilePath $installer.FullName -ArgumentList @("/S") -Label "Stock NSIS install"

if (-not (Test-Path -LiteralPath $uninstallRegistryPath)) {
  throw "Stock NSIS install did not publish its uninstall registry entry."
}
$installed = Get-ItemProperty -LiteralPath $uninstallRegistryPath
$installLocation = ([string]$installed.InstallLocation).Trim('"')
$mainBinaryName = [string]$installed.MainBinaryName
if ([string]::IsNullOrWhiteSpace($installLocation) -or [string]::IsNullOrWhiteSpace($mainBinaryName)) {
  throw "The stock NSIS registry entry is missing InstallLocation or MainBinaryName."
}
$installLocation = [System.IO.Path]::GetFullPath($installLocation)
if ($installLocation -ine [System.IO.Path]::GetFullPath($expectedInstallLocation)) {
  throw "Stock NSIS installed to an unexpected location: $installLocation"
}
$appExecutable = Join-Path $installLocation $mainBinaryName
$uninstaller = Join-Path $installLocation "uninstall.exe"
foreach ($path in @($appExecutable, $uninstaller)) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Stock NSIS install did not create $path."
  }
}

$resourceHashes = [ordered]@{}
foreach ($resourceName in @("THIRD_PARTY_NOTICES.md", "THIRD_PARTY_PROVENANCE.json")) {
  $expectedResource = Join-Path $repoRoot $resourceName
  $installedResource = Join-Path $installLocation $resourceName
  foreach ($path in @($expectedResource, $installedResource)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
      throw "Required release resource is missing: $path"
    }
  }
  $expectedResourceSha256 = (Get-FileHash -LiteralPath $expectedResource -Algorithm SHA256).Hash
  $installedResourceSha256 = (Get-FileHash -LiteralPath $installedResource -Algorithm SHA256).Hash
  if ($installedResourceSha256 -cne $expectedResourceSha256) {
    throw "Installed $resourceName does not match the reviewed repository input."
  }
  $resourceHashes[$resourceName] = $installedResourceSha256
}

$appProcess = Start-Process -FilePath $appExecutable -PassThru
try {
  $launchTimer = [Diagnostics.Stopwatch]::StartNew()
  $logPath = Join-Path $appDataRoot "logs\yap.log"
  while (-not (Test-Path -LiteralPath $logPath -PathType Leaf)) {
    if ($appProcess.HasExited) {
      throw "The installed Yap process exited before creating its canonical Tauri app-data log."
    }
    if ($launchTimer.Elapsed.TotalSeconds -ge $LaunchTimeoutSeconds) {
      throw "Installed Yap did not create $logPath within $LaunchTimeoutSeconds seconds."
    }
    Start-Sleep -Milliseconds 100
  }
} finally {
  if (-not $appProcess.HasExited) {
    $appProcess.Kill($true)
    if (-not $appProcess.WaitForExit(10000)) {
      throw "The installed Yap process tree could not be reaped."
    }
  }
  $appProcess.Dispose()
}

Invoke-BoundedProcess -FilePath $uninstaller -ArgumentList @("/S") -Label "Stock NSIS uninstall"

if (Test-Path -LiteralPath $uninstallRegistryPath) {
  throw "Stock NSIS uninstall left its uninstall registry entry behind."
}
if (Test-Path -LiteralPath $installLocation) {
  throw "Stock NSIS uninstall left its install directory behind."
}
if (-not (Test-Path -LiteralPath $appDataRoot -PathType Container)) {
  throw "Stock silent uninstall unexpectedly removed the canonical app-data directory."
}
if (-not (Test-Path -LiteralPath $productRegistryPath)) {
  throw "Stock silent uninstall unexpectedly removed its preserved product registry key."
}
$preservedInstallLocation = [string](Get-Item -LiteralPath $productRegistryPath).GetValue("")
if (
  [string]::IsNullOrWhiteSpace($preservedInstallLocation) -or
  [System.IO.Path]::GetFullPath($preservedInstallLocation) -ine $installLocation
) {
  throw "Stock silent uninstall did not preserve the expected product install-location record."
}

$afterSha256 = (Get-FileHash -LiteralPath $installer.FullName -Algorithm SHA256).Hash
if ($afterSha256 -cne $beforeSha256) {
  throw "The NSIS installer changed during lifecycle verification."
}

$result = [ordered]@{
  schemaVersion = 1
  installer = $installer.FullName
  installerSha256 = $afterSha256
  appDataRoot = $appDataRoot
  installedResourceSha256 = $resourceHashes
  stockSilentUninstallPreservedAppData = $true
  stockSilentUninstallPreservedProductRegistry = $true
  lifecycleBoundary = if ($isHostedDisposable) { "github-hosted" } else { "explicit-disposable-windows" }
}
$resultJson = $result | ConvertTo-Json -Depth 4
$resultRoot = Join-Path $desktopRoot "tests\results\nsis-smoke\$([Guid]::NewGuid().ToString('N'))"
$null = New-Item -ItemType Directory -Path $resultRoot
$resultJson | Set-Content -LiteralPath (Join-Path $resultRoot "evidence.json") -Encoding utf8
$resultJson
