#requires -Version 7.4
#requires -PSEdition Core

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$modulePath = Join-Path $PSScriptRoot "nsis-smoke-helpers.psm1"
$smokePath = Join-Path $PSScriptRoot "smoke-nsis.ps1"
Import-Module $modulePath -Force

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

function Get-CommandNames([string]$Path) {
  $tokens = $null
  $errors = $null
  $ast = [Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
  if ($errors.Count -ne 0) {
    throw "PowerShell parser errors in '$Path': $($errors.Message -join '; ')"
  }
  return @($ast.FindAll({
    param($Node)
    $Node -is [Management.Automation.Language.CommandAst]
  }, $true) | ForEach-Object { $_.GetCommandName() } | Where-Object { $null -ne $_ })
}

$module = Get-Module | Where-Object { $_.Path -ceq $modulePath } | Select-Object -First 1
Assert-True ($null -ne $module) "NSIS helper module was not loaded."

foreach ($requiredExport in @(
  "Assert-InstallerCleanupAuthorized",
  "Invoke-ContainedProcess"
)) {
  Assert-True $module.ExportedCommands.ContainsKey($requiredExport) "Missing lease adapter export '$requiredExport'."
}

foreach ($removedExport in @(
  "Get-ProcessTreeIds",
  "Invoke-ProcessWithDeadline",
  "Start-ProcessWithEnvironment",
  "Stop-ProcessTreeBounded"
)) {
  Assert-True (-not $module.ExportedCommands.ContainsKey($removedExport)) "Legacy lifecycle export '$removedExport' remains."
}

$smokeCommands = @(Get-CommandNames -Path $smokePath)
$legacySmokeCommands = @(
  "Get-ProcessTreeIds",
  "Invoke-ProcessWithDeadline",
  "Start-ProcessWithEnvironment",
  "Stop-ProcessTreeBounded",
  "Stop-Process",
  "Wait-Process"
)
foreach ($legacyCommand in $legacySmokeCommands) {
  Assert-True ($smokeCommands -notcontains $legacyCommand) "Smoke orchestration still calls legacy lifecycle command '$legacyCommand'."
}
Assert-True (
  @($smokeCommands | Where-Object { $_ -ceq "Invoke-SmokeContainedProcess" }).Count -eq 6
) "Exactly six installer/application/uninstaller consumers must use the cleanup-gated lease adapter."
Assert-True (
  @($smokeCommands | Where-Object { $_ -ceq "Invoke-ContainedProcess" }).Count -eq 1
) "The smoke-local cleanup gate must have exactly one call into the module lease adapter."
Assert-True (
  @($smokeCommands | Where-Object { $_ -ceq "Assert-InstallerCleanupAuthorized" }).Count -ge 1
) "Installer-owned cleanup is not guarded by the fail-closed gateway."

Assert-Throws {
  Assert-InstallerCleanupAuthorized -Authorized:$false -Operation "helper test mutation"
} "blocked|not proven"
Assert-InstallerCleanupAuthorized -Authorized:$true -Operation "helper test mutation"

$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-helper-test-$PID"
$externalRoot = Get-ValidatedChildPath -Root $tempRoot -Token "yap-nsis-helper-external-$PID"

try {
  Initialize-ValidatedTree -Root $tempRoot -Candidate $testRoot | Out-Null
  Initialize-ValidatedTree -Root $tempRoot -Candidate $externalRoot | Out-Null

  Assert-True (Test-StrictChildPath -Root $tempRoot -Candidate $testRoot) "Expected strict child path."
  Assert-True (-not (Test-StrictChildPath -Root $testRoot -Candidate $testRoot)) "Root is not its own child."
  Assert-True (-not (Test-StrictChildPath -Root $testRoot -Candidate "$testRoot-sibling")) "Sibling prefix escaped containment."
  Assert-Throws { Get-ValidatedChildPath -Root $tempRoot -Token "..\escape" } "Unsafe path token"
  Assert-Throws { Get-ValidatedChildPath -Root $tempRoot -Token ("x" * 65) } "Unsafe path token"

  $hashFixture = Join-Path $testRoot "sha256.txt"
  [IO.File]::WriteAllText($hashFixture, "abc")
  Assert-True (
    (Get-Sha256Hex -Path $hashFixture) -ceq "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
  ) "SHA-256 helper returned the wrong digest."

  $nsisRoot = Join-Path $testRoot "nsis-cache"
  $nsisBin = Join-Path $nsisRoot "Bin"
  New-Item -ItemType Directory -Force $nsisBin | Out-Null
  $nsisLauncher = Join-Path $nsisRoot "makensis.exe"
  $nsisCompiler = Join-Path $nsisBin "makensis.exe"
  [IO.File]::WriteAllBytes($nsisLauncher, [byte[]]@(1))
  [IO.File]::WriteAllBytes($nsisCompiler, [byte[]]@(2))
  $nsisTools = Get-TauriNsisToolPaths -Root $nsisRoot
  Assert-True ($nsisTools.LauncherPath -ceq $nsisLauncher) "NSIS launcher path was not deterministic."
  Assert-True ($nsisTools.CompilerPath -ceq $nsisCompiler) "NSIS compiler path was not deterministic."
  Remove-Item -LiteralPath $nsisCompiler -Force -ErrorAction Stop
  Assert-Throws { Get-TauriNsisToolPaths -Root $nsisRoot } "compiler.*missing|missing.*compiler"
  [IO.File]::WriteAllBytes($nsisCompiler, [byte[]]@(2))

  [IO.Directory]::Delete($nsisBin, $true)
  New-Item -ItemType Junction -Path $nsisBin -Target $externalRoot | Out-Null
  Assert-Throws { Get-TauriNsisToolPaths -Root $nsisRoot } "Reparse point"
  [IO.Directory]::Delete($nsisBin, $false)

  $junction = Join-Path $testRoot "junction"
  New-Item -ItemType Junction -Path $junction -Target $externalRoot | Out-Null
  Assert-Throws { Assert-NoReparsePoints -Path $testRoot } "Reparse point"
  [IO.Directory]::Delete($junction, $false)
  Assert-True (Test-Path -LiteralPath $externalRoot -PathType Container) "Junction cleanup touched its target."

  $quickStdout = Join-Path $testRoot "quick.stdout.log"
  $quickStderr = Join-Path $testRoot "quick.stderr.log"
  $quick = Invoke-ContainedProcess `
    -FilePath $env:ComSpec `
    -ArgumentList @("/d", "/s", "/c", "echo helper-stdout & echo helper-stderr 1>&2 & exit /b 7") `
    -TimeoutSeconds 5 `
    -StdoutPath $quickStdout `
    -StderrPath $quickStderr `
    -WorkingDirectory $testRoot
  Assert-True $quick.CleanupProven "Lease adapter did not prove cleanup."
  Assert-True (-not $quick.TimedOut) "Quick lease adapter process timed out."
  Assert-True ($quick.ExitCode -eq 7) "Lease adapter lost the unsigned exit code."
  Assert-True ($quick.RootProcessId -gt 0) "Lease adapter omitted scalar process identity."
  Assert-True ($quick.RootCreationFileTime -gt 0) "Lease adapter omitted scalar creation identity."
  Assert-True ($quick.RootExecutablePath -ieq $env:ComSpec) "Lease adapter reported the wrong executable."
  Assert-True ([IO.File]::ReadAllText($quickStdout) -match "helper-stdout") "Lease adapter lost stdout."
  Assert-True ([IO.File]::ReadAllText($quickStderr) -match "helper-stderr") "Lease adapter lost stderr."

  $deleteRoot = Get-ValidatedChildPath -Root $testRoot -Token "delete-me"
  Initialize-ValidatedTree -Root $testRoot -Candidate $deleteRoot | Out-Null
  Set-Content -LiteralPath (Join-Path $deleteRoot "evidence.txt") -Value "bounded"
  Remove-ValidatedTree -Root $testRoot -Candidate $deleteRoot
  Assert-True (-not (Test-Path -LiteralPath $deleteRoot)) "Validated recursive cleanup left its tree."

  $swapRoot = Get-ValidatedChildPath -Root $testRoot -Token "swap-before-delete"
  Initialize-ValidatedTree -Root $testRoot -Candidate $swapRoot | Out-Null
  $swapTarget = Get-ValidatedChildPath -Root $externalRoot -Token "swap-target"
  Initialize-ValidatedTree -Root $externalRoot -Candidate $swapTarget | Out-Null
  Set-Content -LiteralPath (Join-Path $swapTarget "must-survive.txt") -Value "keep"
  Assert-Throws {
    Remove-ValidatedTree -Root $testRoot -Candidate $swapRoot -BeforeQuarantineRevalidation {
      param($QuarantinePath)
      Remove-Item -LiteralPath $QuarantinePath -Recurse -Force
      New-Item -ItemType Junction -Path $QuarantinePath -Target $swapTarget | Out-Null
    }
  } "Reparse point|redirect|quarantine"
  Assert-True (Test-Path -LiteralPath (Join-Path $swapTarget "must-survive.txt")) "Quarantine swap touched its target."
  $swapQuarantine = Join-Path $testRoot ".swap-before-delete.delete-quarantine"
  if (Test-Path -LiteralPath $swapQuarantine) { [IO.Directory]::Delete($swapQuarantine, $false) }
  Remove-ValidatedTree -Root $externalRoot -Candidate $swapTarget

  $lockRoot = Get-ValidatedChildPath -Root $testRoot -Token "profile-lock"
  New-Item -ItemType Directory -Force $lockRoot | Out-Null
  $deniedMutexFactory = { param($Name) throw [UnauthorizedAccessException]::new("global namespace denied") }
  $firstLock = Enter-SmokeRunLock -ProductKey "Yap.Test" -ProfileRoot $lockRoot -MutexFactory $deniedMutexFactory
  try {
    Assert-True ($firstLock.Kind -ceq "ProfileFile") "Denied Global mutex did not use the profile-lock fallback."
    Assert-Throws {
      Enter-SmokeRunLock -ProductKey "Yap.Test" -ProfileRoot $lockRoot -MutexFactory $deniedMutexFactory
    } "already owns"
  } finally {
    Exit-SmokeRunLock -Lock $firstLock
  }

  $unownedRoot = Get-ValidatedChildPath -Root $testRoot -Token "unowned"
  New-Item -ItemType Directory -Force $unownedRoot | Out-Null
  Assert-Throws { Remove-ValidatedTree -Root $testRoot -Candidate $unownedRoot } "test-data sentinel"
  Remove-Item -LiteralPath $unownedRoot -Force -ErrorAction Stop

  Assert-NoProcessesUnderPath -Root $testRoot
  Write-Output "NSIS smoke helper tests passed."
} finally {
  foreach ($candidate in @($testRoot, $externalRoot)) {
    if (Test-Path -LiteralPath $candidate) {
      Assert-NoProcessesUnderPath -Root $candidate
      Remove-ValidatedTree -Root $tempRoot -Candidate $candidate
    }
  }
}
