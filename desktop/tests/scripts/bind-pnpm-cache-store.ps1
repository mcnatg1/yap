#requires -Version 7.4
#requires -PSEdition Core

$ErrorActionPreference = "Stop"
$localAppData = [Environment]::GetFolderPath(
  [Environment+SpecialFolder]::LocalApplicationData
)
$expectedStore = [IO.Path]::GetFullPath(
  (Join-Path $localAppData "pnpm\store\v11")
)
$cacheStore = [IO.Path]::GetFullPath(
  (Join-Path $HOME "AppData\Local\pnpm\store\v11")
)
if ($expectedStore -ine $cacheStore) {
  throw "The reviewed pnpm cache path does not match Windows LocalApplicationData."
}
$env:PNPM_CONFIG_STORE_DIR = $expectedStore
$actualStoreOutput = @(pnpm store path)
if ($LASTEXITCODE -ne 0 -or $actualStoreOutput.Count -ne 1) {
  throw "Failed to resolve the configured pnpm dependency store."
}
$actualStore = [IO.Path]::GetFullPath(([string]$actualStoreOutput[0]).Trim())
if ($actualStore -ine $expectedStore) {
  throw "pnpm did not accept the reviewed dependency store."
}
"PNPM_CONFIG_STORE_DIR=$expectedStore" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
