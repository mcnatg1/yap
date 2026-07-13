#requires -Version 7.4
#requires -PSEdition Core

function Wait-WdioUniqueWindowCandidate {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory)]
    [scriptblock]$Probe,

    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string]$Description,

    [ValidateRange(1, 1000)]
    [int]$MaxAttempts = 1,

    [ValidateRange(0, 60000)]
    [int]$PollIntervalMilliseconds = 0
  )

  $candidateCount = 0
  for ($attempt = 1; $attempt -le $MaxAttempts; $attempt += 1) {
    $candidateCount = & $Probe
    if ($candidateCount -eq 1) {
      return $candidateCount
    }
    if ($candidateCount -gt 1) {
      throw "Expected exactly one $Description; found $candidateCount."
    }
    if ($candidateCount -lt 0) {
      throw "Candidate probe for $Description returned invalid count $candidateCount."
    }
    if ($attempt -lt $MaxAttempts -and $PollIntervalMilliseconds -gt 0) {
      Start-Sleep -Milliseconds $PollIntervalMilliseconds
    }
  }

  throw "Expected exactly one $Description after $MaxAttempts attempts; found $candidateCount on the final attempt."
}

Export-ModuleMember -Function Wait-WdioUniqueWindowCandidate
