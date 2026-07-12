$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$desktopRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$targetRoot = Join-Path $desktopRoot "src-tauri\target-test"
$previousTarget = [Environment]::GetEnvironmentVariable("CARGO_TARGET_DIR", "Process")
try {
  [Environment]::SetEnvironmentVariable("CARGO_TARGET_DIR", $targetRoot, "Process")
  & pnpm tauri build --bundles nsis --config src-tauri/tauri.test.conf.json
  if ($LASTEXITCODE -ne 0) {
    throw "Yap.Test NSIS build failed with exit code $LASTEXITCODE."
  }
} finally {
  [Environment]::SetEnvironmentVariable("CARGO_TARGET_DIR", $previousTarget, "Process")
}
