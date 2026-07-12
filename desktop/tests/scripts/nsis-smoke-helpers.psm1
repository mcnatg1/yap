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

  $tracked = [System.Collections.Generic.HashSet[int]]::new()
  [void]$tracked.Add($RootProcessId)
  $snapshot = @(Get-ProcessSnapshot)
  Update-TrackedProcessIds -TrackedProcessIds $tracked -Snapshot $snapshot
  return @($tracked | Sort-Object)
}

function Get-ProcessSnapshot {
  return @(
    Get-CimInstance Win32_Process -ErrorAction Stop | ForEach-Object {
      [pscustomobject]@{
        ProcessId = [int]$_.ProcessId
        ParentProcessId = [int]$_.ParentProcessId
        ExecutablePath = $_.ExecutablePath
      }
    }
  )
}

function Update-TrackedProcessIds {
  param(
    [Parameter(Mandatory)][System.Collections.Generic.HashSet[int]]$TrackedProcessIds,
    [Parameter(Mandatory)][object[]]$Snapshot
  )

  $changed = $true
  while ($changed) {
    $changed = $false
    foreach ($process in $Snapshot) {
      $processId = [int]$process.ProcessId
      $parentProcessId = [int]$process.ParentProcessId
      if (
        $processId -le 0 -or
        $processId -eq $PID -or
        $TrackedProcessIds.Contains($processId) -or
        -not $TrackedProcessIds.Contains($parentProcessId)
      ) {
        continue
      }
      [void]$TrackedProcessIds.Add($processId)
      $changed = $true
    }
  }
}

function Get-TrackedProcessDepth {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][hashtable]$ParentById,
    [Parameter(Mandatory)][System.Collections.Generic.HashSet[int]]$TrackedProcessIds
  )

  $depth = 0
  $current = $ProcessId
  $visited = [System.Collections.Generic.HashSet[int]]::new()
  while ($ParentById.ContainsKey($current) -and $visited.Add($current)) {
    $parent = [int]$ParentById[$current]
    if (-not $TrackedProcessIds.Contains($parent)) { break }
    $depth++
    $current = $parent
  }
  return $depth
}

function Stop-TrackedProcessesBounded {
  param(
    [Parameter(Mandatory)][int[]]$ProcessIds,
    [double]$TimeoutSeconds = 5,
    [int]$QuiescencePasses = 2
  )

  if ($TimeoutSeconds -le 0) { throw "Process termination timeout must be positive." }
  if ($QuiescencePasses -lt 2) { throw "Process cleanup requires at least two quiescence passes." }
  $tracked = [System.Collections.Generic.HashSet[int]]::new()
  foreach ($processId in $ProcessIds) {
    if ($processId -le 0 -or $processId -eq $PID) {
      throw "Invalid process ID for bounded cleanup: $processId"
    }
    [void]$tracked.Add($processId)
  }
  if ($tracked.Count -eq 0) { throw "Bounded cleanup requires at least one process ID." }

  $terminated = [System.Collections.Generic.HashSet[int]]::new()
  $terminationErrors = [System.Collections.Generic.List[string]]::new()
  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  $iterations = 0
  $quietPasses = 0
  do {
    $iterations++
    $snapshot = @(Get-ProcessSnapshot)
    Update-TrackedProcessIds -TrackedProcessIds $tracked -Snapshot $snapshot
    $alive = @($snapshot | Where-Object { $tracked.Contains([int]$_.ProcessId) })
    if ($alive.Count -eq 0) {
      $quietPasses++
      if ($quietPasses -ge $QuiescencePasses) {
        return [pscustomobject]@{
          DiscoveredProcessIds = @($tracked | Sort-Object)
          TerminatedProcessIds = @($terminated | Sort-Object)
          ResidualProcessIds = @()
          TerminationErrors = @($terminationErrors)
          Iterations = $iterations
          QuiescencePasses = $quietPasses
        }
      }
    } else {
      $quietPasses = 0
      $parentById = @{}
      foreach ($process in $snapshot) {
        $parentById[[int]$process.ProcessId] = [int]$process.ParentProcessId
      }
      $ordered = @(
        $alive | Sort-Object {
          -(Get-TrackedProcessDepth `
            -ProcessId ([int]$_.ProcessId) `
            -ParentById $parentById `
            -TrackedProcessIds $tracked)
        }
      )
      foreach ($process in $ordered) {
        $processId = [int]$process.ProcessId
        try {
          Stop-Process -Id $processId -Force -ErrorAction Stop
          [void]$terminated.Add($processId)
        } catch {
          if (Test-ProcessAlive -ProcessId $processId) {
            $terminationErrors.Add("${processId}:$($_.Exception.Message)")
          }
        }
      }
    }
    Start-Sleep -Milliseconds 50
  } while ([DateTime]::UtcNow -lt $deadline)

  $snapshot = @(Get-ProcessSnapshot)
  Update-TrackedProcessIds -TrackedProcessIds $tracked -Snapshot $snapshot
  $residuals = @(
    $snapshot |
      Where-Object { $tracked.Contains([int]$_.ProcessId) } |
      ForEach-Object { [int]$_.ProcessId } |
      Sort-Object
  )
  $report = [ordered]@{
    discoveredProcessIds = @($tracked | Sort-Object)
    terminatedProcessIds = @($terminated | Sort-Object)
    residualProcessIds = $residuals
    terminationErrors = @($terminationErrors)
    iterations = $iterations
    quiescencePasses = $quietPasses
  }
  throw "Process cleanup did not reach quiescence: $($report | ConvertTo-Json -Compress -Depth 4)"
}

function Stop-ProcessTreeBounded {
  param(
    [Parameter(Mandatory)][int]$RootProcessId,
    [int[]]$SeedProcessIds = @(),
    [double]$TimeoutSeconds = 5
  )

  return Stop-TrackedProcessesBounded `
    -ProcessIds (@($RootProcessId) + @($SeedProcessIds)) `
    -TimeoutSeconds $TimeoutSeconds
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
  $tracked = [System.Collections.Generic.HashSet[int]]::new()
  [void]$tracked.Add($process.Id)
  $quietPasses = 0
  $iterations = 0
  do {
    $iterations++
    $snapshot = @(Get-ProcessSnapshot)
    Update-TrackedProcessIds -TrackedProcessIds $tracked -Snapshot $snapshot
    $alive = @($snapshot | Where-Object { $tracked.Contains([int]$_.ProcessId) })
    $process.Refresh()
    if ($process.HasExited -and $alive.Count -eq 0) {
      $quietPasses++
      if ($quietPasses -ge 2) { break }
    } else {
      $quietPasses = 0
    }
    Start-Sleep -Milliseconds 50
  } while ([DateTime]::UtcNow -lt $deadline)

  if (-not $process.HasExited -or $quietPasses -lt 2) {
    $cleanupEvidence = "cleanup was not attempted"
    try {
      $cleanup = Stop-ProcessTreeBounded `
        -RootProcessId $process.Id `
        -SeedProcessIds @($tracked) `
        -TimeoutSeconds 5
      $cleanupEvidence = $cleanup | ConvertTo-Json -Compress -Depth 4
    } catch {
      $cleanupEvidence = $_.Exception.Message
    }
    throw "Process $($process.Id) or its descendants exceeded the $TimeoutSeconds second deadline. Cleanup evidence: $cleanupEvidence"
  }

  $process.WaitForExit()
  return [pscustomobject]@{
    ProcessId = $process.Id
    ProcessIds = @($tracked | Sort-Object)
    ExitCode = $process.ExitCode
    DurationMs = [int]([DateTime]::UtcNow - $startedAt).TotalMilliseconds
    DiscoveryIterations = $iterations
    QuiescencePasses = $quietPasses
    ResidualProcessIds = @()
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
  Stop-TrackedProcessesBounded, `
  Test-ProcessAlive, `
  Test-StrictChildPath, `
  Wait-PathAbsent
