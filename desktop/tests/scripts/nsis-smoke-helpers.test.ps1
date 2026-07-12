$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "nsis-smoke-helpers.psm1") -Force

function Assert-True([bool]$Condition, [string]$Message) {
  if (-not $Condition) { throw $Message }
}

function Assert-Throws([scriptblock]$Operation, [string]$Pattern) {
  try {
    & $Operation
  } catch {
    if ($_.Exception.Message -notmatch $Pattern) {
      throw "Expected error matching '$Pattern', received '$($_.Exception.Message)'."
    }
    return
  }
  throw "Expected operation to throw an error matching '$Pattern'."
}

$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$testRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-helper-test-$PID"
$externalRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-helper-external-$PID"
$processRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-helper-process-$PID"

try {
  New-Item -ItemType Directory -Force $testRoot, $externalRoot, $processRoot | Out-Null

  Assert-True (Test-StrictChildPath -Root $tempRoot -Candidate $testRoot) "Expected strict child path."
  Assert-True (-not (Test-StrictChildPath -Root $testRoot -Candidate $testRoot)) "Root is not its own child."
  Assert-True (-not (Test-StrictChildPath -Root $testRoot -Candidate "$testRoot-sibling")) "Sibling prefix escaped containment."
  Assert-Throws { Get-ValidatedChildPath -Root $tempRoot -Token "..\escape" } "Unsafe path token"
  Assert-Throws { Get-ValidatedChildPath -Root $tempRoot -Token ("x" * 65) } "Unsafe path token"

  $junction = Join-Path $testRoot "junction"
  New-Item -ItemType Junction -Path $junction -Target $externalRoot | Out-Null
  Assert-Throws { Assert-NoReparsePoints -Path $testRoot } "Reparse point"
  Remove-Item -LiteralPath $junction -Force -ErrorAction Stop
  Assert-True (Test-Path -LiteralPath $externalRoot -PathType Container) "Junction cleanup touched its target."

  $quickOut = Join-Path $processRoot "quick.out.log"
  $quickErr = Join-Path $processRoot "quick.err.log"
  $quick = Invoke-ProcessWithDeadline `
    -FilePath "cmd.exe" `
    -ArgumentList @("/d", "/c", "exit 7") `
    -TimeoutSeconds 5 `
    -StdoutPath $quickOut `
    -StderrPath $quickErr
  Assert-True ($quick.ExitCode -eq 7) "Deadline helper lost the process exit code."
  Assert-True ($quick.ProcessIds -contains $quick.ProcessId) "Deadline helper omitted its root process evidence."
  Assert-True ($quick.QuiescencePasses -ge 2) "Deadline helper did not verify process-tree quiescence."

  $timeoutPidPath = Join-Path $processRoot "timeout.pid"
  $timeoutScript = "`$PID | Set-Content -LiteralPath '$timeoutPidPath' -Encoding ascii; Start-Sleep -Seconds 30"
  $timeoutEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($timeoutScript))
  Assert-Throws {
    Invoke-ProcessWithDeadline `
      -FilePath "powershell.exe" `
      -ArgumentList @("-NoProfile", "-EncodedCommand", $timeoutEncoded) `
      -TimeoutSeconds 1.5 `
      -StdoutPath (Join-Path $processRoot "timeout.out.log") `
      -StderrPath (Join-Path $processRoot "timeout.err.log")
  } "exceeded the 1.5 second deadline"
  Assert-True (Test-Path -LiteralPath $timeoutPidPath -PathType Leaf) "Timed process did not report its PID."
  $timedProcessId = [int](Get-Content -LiteralPath $timeoutPidPath -Raw)
  Assert-True (-not (Test-ProcessAlive -ProcessId $timedProcessId)) "Timed process survived deadline cleanup."

  $childIdPath = Join-Path $processRoot "child.pid"
  $childScript = @"
`$child = Start-Process powershell.exe -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30') -PassThru
`$child.Id | Set-Content -LiteralPath '$childIdPath' -Encoding ascii
Start-Sleep -Seconds 30
"@
  $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childScript))
  $parent = Start-Process powershell.exe -ArgumentList @("-NoProfile", "-EncodedCommand", $encoded) -PassThru -WindowStyle Hidden
  $deadline = [DateTime]::UtcNow.AddSeconds(5)
  while (-not (Test-Path -LiteralPath $childIdPath) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
  }
  Assert-True (Test-Path -LiteralPath $childIdPath -PathType Leaf) "Child process did not report its PID."
  $childId = [int](Get-Content -LiteralPath $childIdPath -Raw)
  $tree = @(Get-ProcessTreeIds -RootProcessId $parent.Id)
  Assert-True ($tree -contains $childId) "Process-tree discovery omitted the child."
  $cleanup = Stop-ProcessTreeBounded -RootProcessId $parent.Id -TimeoutSeconds 5
  Assert-True ($cleanup.DiscoveredProcessIds -contains $childId) "Cleanup evidence omitted the child."
  Assert-True ($cleanup.QuiescencePasses -ge 2) "Cleanup did not wait for repeated quiescence."
  Assert-True (-not (Test-ProcessAlive -ProcessId $parent.Id)) "Parent process survived bounded termination."
  Assert-True (-not (Test-ProcessAlive -ProcessId $childId)) "Child process survived bounded termination."

  $deleteRoot = Get-ValidatedChildPath -Root $testRoot -Token "delete-me"
  New-Item -ItemType Directory -Force $deleteRoot | Out-Null
  Set-Content -LiteralPath (Join-Path $deleteRoot "evidence.txt") -Value "bounded"
  Remove-ValidatedTree -Root $testRoot -Candidate $deleteRoot
  Assert-True (-not (Test-Path -LiteralPath $deleteRoot)) "Validated recursive cleanup left its tree."

  Write-Output "NSIS smoke helper tests passed."
} finally {
  foreach ($candidate in @($testRoot, $externalRoot, $processRoot)) {
    if (Test-Path -LiteralPath $candidate) {
      Assert-NoProcessesUnderPath -Root $candidate
      Remove-ValidatedTree -Root $tempRoot -Candidate $candidate
    }
  }
}
