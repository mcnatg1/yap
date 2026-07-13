#requires -Version 7.4
#requires -PSEdition Core

param(
  [Parameter(Mandatory)]
  [ValidateSet("Sleep", "Descendant", "Io", "Nested")]
  [string]$Mode,

  [string]$ChildPidPath,
  [string]$ContainedProcessSource,
  [string]$NestedResultPath,
  [string]$NestedStdoutPath,
  [string]$NestedStderrPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

switch ($Mode) {
  "Sleep" {
    Start-Sleep -Seconds 30
  }

  "Descendant" {
    if ([string]::IsNullOrWhiteSpace($ChildPidPath)) {
      throw "Descendant mode requires ChildPidPath."
    }
    $child = Start-Process `
      -FilePath ([Environment]::ProcessPath) `
      -ArgumentList @("-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 30") `
      -PassThru `
      -WindowStyle Hidden
    try {
      [IO.File]::WriteAllText($ChildPidPath, $child.Id.ToString([Globalization.CultureInfo]::InvariantCulture))
    } finally {
      $child.Dispose()
    }
    Start-Sleep -Seconds 30
  }

  "Io" {
    [Console]::Out.WriteLine("fixture-stdout")
    [Console]::Out.WriteLine("cwd=$([Environment]::CurrentDirectory)")
    [Console]::Out.WriteLine("override=$([Environment]::GetEnvironmentVariable('YAP_CONTAINED_OVERRIDE', 'Process'))")
    [Console]::Out.WriteLine("removed=$([Environment]::GetEnvironmentVariable('YAP_CONTAINED_REMOVE', 'Process'))")
    [Console]::Error.WriteLine("fixture-stderr")
  }

  "Nested" {
    foreach ($required in @($ContainedProcessSource, $NestedResultPath, $NestedStdoutPath, $NestedStderrPath)) {
      if ([string]::IsNullOrWhiteSpace($required)) {
        throw "Nested mode requires all nested-process paths."
      }
    }

    Add-Type -Path $ContainedProcessSource
    $request = [Yap.NsisSmoke.LaunchRequest]::Create(
      $env:ComSpec,
      [string[]]@("/d", "/s", "/c", "exit /b 0"),
      $NestedStdoutPath,
      $NestedStderrPath,
      [IO.Path]::GetDirectoryName($NestedResultPath),
      [Collections.Hashtable]@{}
    )
    $lease = [Yap.NsisSmoke.WindowsContainedProcessLauncher]::new().Launch($request)
    try {
      $root = $lease.WaitForRootExit([TimeSpan]::FromSeconds(5))
      if (-not $root.Exited -or $root.ExitCode -ne 0) {
        throw "Nested contained process did not exit successfully."
      }
      $quiescence = $lease.WaitForQuiescence([TimeSpan]::FromSeconds(5))
      if (-not $quiescence.Quiescent) {
        throw "Nested contained Job did not become quiescent."
      }
      [IO.File]::WriteAllText($NestedResultPath, "nested-ok")
    } finally {
      $lease.Dispose()
    }
  }
}
