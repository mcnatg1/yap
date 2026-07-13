#requires -Version 7.4
#requires -PSEdition Core

$ErrorActionPreference = "Stop"

Import-Module -Name (Join-Path $PSScriptRoot "..\wdio\native-window-recovery.psm1") -Force

function Assert-Equal {
  param(
    [Parameter(Mandatory)]$Actual,
    [Parameter(Mandatory)]$Expected,
    [Parameter(Mandatory)][string]$Message
  )
  if ($Actual -ne $Expected) {
    throw "$Message Expected '$Expected', got '$Actual'."
  }
}

$delayedCounts = [System.Collections.Generic.Queue[int]]::new()
$delayedCounts.Enqueue(0)
$delayedCounts.Enqueue(0)
$delayedCounts.Enqueue(1)
$delayedAttempts = 0
$result = Wait-WdioUniqueWindowCandidate `
  -Probe {
    $script:delayedAttempts += 1
    $delayedCounts.Dequeue()
  } `
  -Description "main Yap window" `
  -MaxAttempts 3 `
  -PollIntervalMilliseconds 0
Assert-Equal $result 1 "Delayed window recovery did not return the unique candidate."
Assert-Equal $delayedAttempts 3 "Delayed window recovery did not poll to readiness."

$duplicateCounts = [System.Collections.Generic.Queue[int]]::new()
$duplicateCounts.Enqueue(2)
$duplicateCounts.Enqueue(1)
$duplicateAttempts = 0
$duplicateError = try {
  Wait-WdioUniqueWindowCandidate `
    -Probe {
      $script:duplicateAttempts += 1
      $duplicateCounts.Dequeue()
    } `
    -Description "main Yap window" `
    -MaxAttempts 2 `
    -PollIntervalMilliseconds 0
  $null
} catch {
  $_
}
if ($null -eq $duplicateError -or $duplicateError.Exception.Message -notmatch "found 2") {
  throw "Duplicate native windows did not fail the exact-one invariant."
}
Assert-Equal $duplicateAttempts 1 "Duplicate native windows should fail without another poll."

$missingAttempts = 0
$missingError = try {
  Wait-WdioUniqueWindowCandidate `
    -Probe {
      $script:missingAttempts += 1
      0
    } `
    -Description "main Yap window" `
    -MaxAttempts 3 `
    -PollIntervalMilliseconds 0
  $null
} catch {
  $_
}
if ($null -eq $missingError -or $missingError.Exception.Message -notmatch "3 attempts") {
  throw "Missing native window polling did not stop at the bounded deadline."
}
Assert-Equal $missingAttempts 3 "Missing native window polling exceeded or missed its bound."

Write-Host "Native WDIO window-recovery tests passed."
