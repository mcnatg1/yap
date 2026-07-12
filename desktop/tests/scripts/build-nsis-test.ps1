#requires -Version 7.4
#requires -PSEdition Core

param(
  [ValidateRange(1, 3600)]
  [int]$BuildTimeoutSeconds = 2700
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$desktopRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$targetRoot = Join-Path $desktopRoot "src-tauri\target-test"
$pnpmCommand = Get-Command "pnpm" -CommandType Application -ErrorAction Stop |
  Where-Object { [System.IO.Path]::GetExtension($_.Source) -in @(".cmd", ".exe") } |
  Select-Object -First 1
if ($null -eq $pnpmCommand) {
  throw "pnpm must resolve to a Windows .cmd or .exe application."
}
$pnpmPath = $pnpmCommand.Source
$buildTimeoutMilliseconds = [int][TimeSpan]::FromSeconds($BuildTimeoutSeconds).TotalMilliseconds
$process = $null
try {
  $process = Start-Process `
    -FilePath $pnpmPath `
    -ArgumentList @("tauri", "build", "--bundles", "nsis", "--config", "src-tauri/tauri.test.conf.json") `
    -WorkingDirectory $desktopRoot `
    -Environment @{ CARGO_TARGET_DIR = $targetRoot } `
    -NoNewWindow `
    -PassThru
  if (-not $process.WaitForExit($buildTimeoutMilliseconds)) {
    throw "Yap.Test NSIS build exceeded the $BuildTimeoutSeconds second deadline."
  }
  if ($process.ExitCode -ne 0) {
    throw "Yap.Test NSIS build failed with exit code $($process.ExitCode)."
  }
} finally {
  if ($null -ne $process) {
    try {
      if (-not $process.HasExited) {
        try {
          $process.Kill($true)
        } catch [InvalidOperationException] {
          if (-not $process.HasExited) { throw }
        }
        if (-not $process.WaitForExit(10000)) {
          throw "Unable to reap the timed-out Yap.Test NSIS build process tree."
        }
      }
    } finally {
      $process.Dispose()
    }
  }
}
