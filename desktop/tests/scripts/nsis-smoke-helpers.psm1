function Assert-SafePathToken {
  param([Parameter(Mandatory)][string]$Token)

  if ($Token -in @(".", "..") -or $Token -notmatch "^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$") {
    throw "Unsafe path token: $Token"
  }
  return $Token
}

function Get-PathRelativeTo {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd("\", "/")
  $candidateFull = [System.IO.Path]::GetFullPath($Candidate)
  $separators = [char[]]@("\", "/")
  $rootParts = @($rootFull.Split($separators, [StringSplitOptions]::RemoveEmptyEntries))
  $candidateParts = @($candidateFull.Split($separators, [StringSplitOptions]::RemoveEmptyEntries))
  $common = 0
  while (
    $common -lt $rootParts.Count -and
    $common -lt $candidateParts.Count -and
    [string]::Equals($rootParts[$common], $candidateParts[$common], [StringComparison]::OrdinalIgnoreCase)
  ) {
    $common++
  }
  if ($common -eq 0) { return $candidateFull }

  $relativeParts = [System.Collections.Generic.List[string]]::new()
  for ($index = $common; $index -lt $rootParts.Count; $index++) { $relativeParts.Add("..") }
  for ($index = $common; $index -lt $candidateParts.Count; $index++) {
    $relativeParts.Add($candidateParts[$index])
  }
  if ($relativeParts.Count -eq 0) { return "." }
  return [string]::Join([System.IO.Path]::DirectorySeparatorChar, $relativeParts)
}

function Test-StrictChildPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $relative = Get-PathRelativeTo -Root $Root -Candidate $Candidate
  if ([string]::IsNullOrWhiteSpace($relative) -or $relative -eq ".") { return $false }
  if ([System.IO.Path]::IsPathRooted($relative)) { return $false }
  $firstSegment = $relative.Split([System.IO.Path]::DirectorySeparatorChar)[0]
  return $firstSegment -ne ".."
}

function Get-ValidatedChildPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Token
  )

  $safeToken = Assert-SafePathToken -Token $Token
  $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root $safeToken))
  if (-not (Test-StrictChildPath -Root $Root -Candidate $candidate)) {
    throw "Path token did not resolve to a strict child of the configured root."
  }
  return $candidate
}

function Assert-PathIsNotReparsePoint {
  param([Parameter(Mandatory)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) { return }
  $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
  if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Reparse point is not allowed in NSIS smoke paths: $($item.FullName)"
  }
}

function Assert-NoReparsePoints {
  param([Parameter(Mandatory)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) { return }
  $pending = [System.Collections.Generic.Stack[string]]::new()
  $pending.Push([System.IO.Path]::GetFullPath($Path))
  while ($pending.Count -gt 0) {
    $current = $pending.Pop()
    $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
      throw "Reparse point is not allowed in NSIS smoke paths: $($item.FullName)"
    }
    if ($item.PSIsContainer) {
      foreach ($child in Get-ChildItem -LiteralPath $item.FullName -Force -ErrorAction Stop) {
        $pending.Push($child.FullName)
      }
    }
  }
}

function Remove-ValidatedTree {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $rootFull = [System.IO.Path]::GetFullPath($Root)
  $candidateFull = [System.IO.Path]::GetFullPath($Candidate)
  if (-not (Test-StrictChildPath -Root $rootFull -Candidate $candidateFull)) {
    throw "Refusing recursive deletion outside a strict child of $rootFull."
  }
  if (-not (Test-Path -LiteralPath $candidateFull)) { return }
  Assert-NoReparsePoints -Path $candidateFull
  Remove-Item -LiteralPath $candidateFull -Recurse -Force -ErrorAction Stop
  if (Test-Path -LiteralPath $candidateFull) {
    throw "Recursive cleanup did not remove $candidateFull."
  }
}

function Test-ProcessAlive {
  param([Parameter(Mandatory)][int]$ProcessId)

  try {
    [void](Get-Process -Id $ProcessId -ErrorAction Stop)
    return $true
  } catch [Microsoft.PowerShell.Commands.ProcessCommandException] {
    return $false
  }
}

function Get-ProcessTreeIds {
  param([Parameter(Mandatory)][int]$RootProcessId)

  $processes = @(Get-CimInstance Win32_Process -ErrorAction Stop)
  $children = @{}
  foreach ($process in $processes) {
    $parentId = [int]$process.ParentProcessId
    if (-not $children.ContainsKey($parentId)) {
      $children[$parentId] = [System.Collections.Generic.List[int]]::new()
    }
    $children[$parentId].Add([int]$process.ProcessId)
  }

  $result = [System.Collections.Generic.List[int]]::new()
  $pending = [System.Collections.Generic.Stack[int]]::new()
  $pending.Push($RootProcessId)
  while ($pending.Count -gt 0) {
    $current = $pending.Pop()
    if ($result.Contains($current)) { continue }
    $result.Add($current)
    if ($children.ContainsKey($current)) {
      foreach ($childId in $children[$current]) { $pending.Push($childId) }
    }
  }
  return @($result)
}

function Stop-ProcessTreeBounded {
  param(
    [Parameter(Mandatory)][int]$RootProcessId,
    [double]$TimeoutSeconds = 5
  )

  if ($TimeoutSeconds -le 0) { throw "Process termination timeout must be positive." }
  $processIds = @(Get-ProcessTreeIds -RootProcessId $RootProcessId)
  [array]::Reverse($processIds)
  foreach ($processId in $processIds) {
    if (-not (Test-ProcessAlive -ProcessId $processId)) { continue }
    try {
      Stop-Process -Id $processId -Force -ErrorAction Stop
    } catch {
      if (Test-ProcessAlive -ProcessId $processId) { throw }
    }
  }

  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  do {
    $alive = @($processIds | Where-Object { Test-ProcessAlive -ProcessId $_ })
    if ($alive.Count -eq 0) { return $processIds }
    Start-Sleep -Milliseconds 50
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Process tree did not terminate before the deadline: $($alive -join ', ')."
}

function Invoke-ProcessWithDeadline {
  param(
    [Parameter(Mandatory)][string]$FilePath,
    [string[]]$ArgumentList = @(),
    [Parameter(Mandatory)][double]$TimeoutSeconds,
    [Parameter(Mandatory)][string]$StdoutPath,
    [Parameter(Mandatory)][string]$StderrPath
  )

  if ($TimeoutSeconds -le 0) { throw "Process timeout must be positive." }
  if ([System.IO.Path]::GetFullPath($StdoutPath) -eq [System.IO.Path]::GetFullPath($StderrPath)) {
    throw "Process stdout and stderr paths must be different."
  }
  New-Item -ItemType Directory -Force ([System.IO.Path]::GetDirectoryName($StdoutPath)) | Out-Null
  New-Item -ItemType Directory -Force ([System.IO.Path]::GetDirectoryName($StderrPath)) | Out-Null

  $process = Start-Process `
    -FilePath $FilePath `
    -ArgumentList $ArgumentList `
    -PassThru `
    -RedirectStandardOutput $StdoutPath `
    -RedirectStandardError $StderrPath `
    -WindowStyle Hidden
  # Windows PowerShell 5.1 loses ExitCode unless the process handle is materialized early.
  [void]$process.Handle
  $startedAt = [DateTime]::UtcNow
  $deadline = $startedAt.AddSeconds($TimeoutSeconds)
  while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
    $process.Refresh()
  }
  if (-not $process.HasExited) {
    $terminationError = $null
    try {
      Stop-ProcessTreeBounded -RootProcessId $process.Id -TimeoutSeconds 5 | Out-Null
    } catch {
      $terminationError = $_.Exception.Message
    }
    $message = "Process $($process.Id) exceeded its $TimeoutSeconds second deadline."
    if ($terminationError) { $message += " Termination also failed: $terminationError" }
    throw $message
  }
  $process.WaitForExit()
  return [pscustomobject]@{
    ProcessId = $process.Id
    ExitCode = $process.ExitCode
    DurationMs = [int]([DateTime]::UtcNow - $startedAt).TotalMilliseconds
  }
}

function Assert-ProcessSurvives {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][double]$DurationSeconds
  )

  if ($DurationSeconds -le 0) { throw "Process survival duration must be positive." }
  $deadline = [DateTime]::UtcNow.AddSeconds($DurationSeconds)
  while ([DateTime]::UtcNow -lt $deadline) {
    if (-not (Test-ProcessAlive -ProcessId $ProcessId)) {
      throw "Process $ProcessId exited before the $DurationSeconds second launch probe completed."
    }
    Start-Sleep -Milliseconds 50
  }
}

function Wait-PathAbsent {
  param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][double]$TimeoutSeconds
  )

  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  while ((Test-Path -LiteralPath $Path) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 100
  }
  if (Test-Path -LiteralPath $Path) {
    throw "Path remained after the $TimeoutSeconds second deadline: $Path"
  }
}

function Get-ProcessesUnderPath {
  param([Parameter(Mandatory)][string]$Root)

  $matches = @()
  foreach ($process in Get-CimInstance Win32_Process -ErrorAction Stop) {
    if ([string]::IsNullOrWhiteSpace($process.ExecutablePath)) { continue }
    if (Test-StrictChildPath -Root $Root -Candidate $process.ExecutablePath) {
      $matches += [pscustomobject]@{
        ProcessId = [int]$process.ProcessId
        ExecutablePath = $process.ExecutablePath
      }
    }
  }
  return $matches
}

function Assert-NoProcessesUnderPath {
  param([Parameter(Mandatory)][string]$Root)

  $matches = @(Get-ProcessesUnderPath -Root $Root)
  if ($matches.Count -gt 0) {
    $footprint = $matches | ForEach-Object { "$($_.ProcessId):$($_.ExecutablePath)" }
    throw "Processes remain under the install root: $($footprint -join ', ')."
  }
}

Export-ModuleMember -Function `
  Assert-NoProcessesUnderPath, `
  Assert-NoReparsePoints, `
  Assert-PathIsNotReparsePoint, `
  Assert-ProcessSurvives, `
  Assert-SafePathToken, `
  Get-ProcessesUnderPath, `
  Get-ProcessTreeIds, `
  Get-ValidatedChildPath, `
  Invoke-ProcessWithDeadline, `
  Remove-ValidatedTree, `
  Stop-ProcessTreeBounded, `
  Test-ProcessAlive, `
  Test-StrictChildPath, `
  Wait-PathAbsent
