#requires -Version 7.4
#requires -PSEdition Core

param(
  [string]$BundleDirectory = "",
  [string]$ExpectedInstallerSha256 = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

& (Join-Path $PSScriptRoot "smoke-nsis.ps1") `
  -Mode IsolatedProductionDelete `
  -BundleDirectory $BundleDirectory `
  -ExpectedInstallerSha256 $ExpectedInstallerSha256
