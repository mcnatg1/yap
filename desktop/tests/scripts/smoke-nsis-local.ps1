#requires -Version 7.4
#requires -PSEdition Core

param(
  [string]$BundleDirectory = "",
  [string]$ExpectedInstallerSha256 = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($BundleDirectory)) {
  $BundleDirectory = Join-Path $PSScriptRoot "..\..\src-tauri\target-test\release\bundle\nsis"
}

& (Join-Path $PSScriptRoot "smoke-nsis.ps1") `
  -Mode LocalSafe `
  -BundleDirectory $BundleDirectory `
  -ExpectedInstallerSha256 $ExpectedInstallerSha256
