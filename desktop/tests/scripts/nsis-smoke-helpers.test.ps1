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

function Assert-FailClosedCatch {
  param(
    [Parameter(Mandatory)]
    [Management.Automation.Language.CatchClauseAst]$CatchClause,
    [Parameter(Mandatory)]
    [string]$Context
  )

  $assignments = @($CatchClause.FindAll({
    param($Node)
    $Node -is [Management.Automation.Language.AssignmentStatementAst]
  }, $true))
  $authorizationAssignments = @($assignments | Where-Object {
    $_.Left.Extent.Text -in @(
      '$script:filesystemCleanupAuthorized',
      '$evidence.cleanupAuthority.authorized'
    )
  })
  Assert-True (
    $authorizationAssignments.Count -eq 2 -and
    @($authorizationAssignments | Where-Object { $_.Right.Extent.Text -cne '$false' }).Count -eq 0
  ) "$Context must revoke both cleanup-authorization fields."
  $retainedPathAssignments = @($assignments | Where-Object {
    $_.Left.Extent.Text -ceq '$evidence.cleanupAuthority.retainedPaths'
  })
  Assert-True (
    $retainedPathAssignments.Count -eq 1 -and
    $retainedPathAssignments[0].Right.Extent.Text -match '\$footprintPaths\.Values' -and
    $retainedPathAssignments[0].Right.Extent.Text -match '\$smokeRoot'
  ) "$Context must retain every protected installer-owned path."
}

$module = Get-Module | Where-Object { $_.Path -ceq $modulePath } | Select-Object -First 1
Assert-True ($null -ne $module) "NSIS helper module was not loaded."

foreach ($requiredExport in @(
  "Assert-InstallerCleanupAuthorized",
  "Invoke-ContainedProcess",
  "Invoke-YapAuthorizedCleanupMutation"
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
  "Get-Process",
  "Start-Process",
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

$smokeTokens = $null
$smokeErrors = $null
$smokeAst = [Management.Automation.Language.Parser]::ParseFile(
  $smokePath,
  [ref]$smokeTokens,
  [ref]$smokeErrors
)
$gatewayCalls = @($smokeAst.FindAll({
  param($Node)
  $Node -is [Management.Automation.Language.CommandAst] -and
  $Node.GetCommandName() -ceq "Invoke-YapAuthorizedCleanupMutation"
}, $true))
foreach ($gatewayCall in $gatewayCalls) {
  $authorizationParameters = @($gatewayCall.CommandElements | Where-Object {
    $_ -is [Management.Automation.Language.CommandParameterAst] -and
    $_.ParameterName -ceq "CleanupAuthorized"
  })
  Assert-True ($authorizationParameters.Count -eq 1) "Cleanup gateway call omitted its single authorization binding."
  $authorization = $authorizationParameters[0].Argument
  Assert-True (
    $authorization -is [Management.Automation.Language.VariableExpressionAst] -and
    $authorization.VariablePath.UserPath -ceq "filesystemCleanupAuthorized"
  ) "Cleanup gateway must bind the authoritative filesystemCleanupAuthorized variable directly."
}
$forbiddenMembers = @($smokeAst.FindAll({
  param($Node)
  $Node -is [Management.Automation.Language.InvokeMemberExpressionAst]
}, $true) | ForEach-Object { $_.Member.Value } | Where-Object { $_ -in @("Kill", "WaitForExit") })
Assert-True ($forbiddenMembers.Count -eq 0) "Smoke orchestration reacquired managed Process lifecycle control."

$smokeLeaseFunctions = @($smokeAst.FindAll({
  param($Node)
  $Node -is [Management.Automation.Language.FunctionDefinitionAst] -and
  $Node.Name -ceq "Invoke-SmokeContainedProcess"
}, $true))
Assert-True ($smokeLeaseFunctions.Count -eq 1) "Smoke orchestration must define exactly one cleanup-gated lease adapter."
$launchCatches = @($smokeLeaseFunctions[0].Body.FindAll({
  param($Node)
  $Node -is [Management.Automation.Language.CatchClauseAst]
}, $true))
Assert-True ($launchCatches.Count -eq 1) "The smoke lease adapter must have exactly one fail-closed catch path."
Assert-FailClosedCatch -CatchClause $launchCatches[0] -Context "A caught launch exception"

$cleanupFinalizerTries = @($smokeAst.FindAll({
  param($Node)
  $Node -is [Management.Automation.Language.TryStatementAst] -and
  $Node.Body.Extent.Text -match '-Phase\s+"cleanup uninstall"'
}, $true))
Assert-True ($cleanupFinalizerTries.Count -eq 1) "Smoke orchestration must have exactly one cleanup-uninstaller finalizer."
Assert-True ($cleanupFinalizerTries[0].CatchClauses.Count -eq 1) "Cleanup-uninstaller finalizer must have one catch path."
Assert-FailClosedCatch `
  -CatchClause $cleanupFinalizerTries[0].CatchClauses[0] `
  -Context "A failed reparse or cleanup-uninstaller finalizer"

$residualAuditTries = @($smokeAst.FindAll({
  param($Node)
  if ($Node -isnot [Management.Automation.Language.TryStatementAst]) { return $false }
  $commands = @($Node.Body.FindAll({
    param($Child)
    $Child -is [Management.Automation.Language.CommandAst] -and
    $Child.GetCommandName() -ceq "Assert-NoProcessesUnderPath"
  }, $true))
  return $commands.Count -eq 1 -and $Node.Body.Statements.Count -eq 1
}, $true))
Assert-True ($residualAuditTries.Count -eq 1) "Smoke orchestration must have exactly one final residual-process audit."
Assert-True ($residualAuditTries[0].CatchClauses.Count -eq 1) "Residual-process audit must have one catch path."
Assert-FailClosedCatch `
  -CatchClause $residualAuditTries[0].CatchClauses[0] `
  -Context "A failed or positive residual-process audit"

Assert-Throws {
  Assert-InstallerCleanupAuthorized -Authorized:$false -Operation "helper test mutation"
} "blocked|not proven"
Assert-InstallerCleanupAuthorized -Authorized:$true -Operation "helper test mutation"
$script:blockedMutationCount = 0
$blockedMutation = Invoke-YapAuthorizedCleanupMutation `
  -CleanupAuthorized:$false `
  -Name "blocked helper mutation" `
  -RetainedPaths @("C:\retained-install-root") `
  -Action { $script:blockedMutationCount++ }
Assert-True (-not $blockedMutation.Executed) "Unauthorized cleanup mutation reported execution."
Assert-True ($script:blockedMutationCount -eq 0) "Unauthorized cleanup mutation action executed."
Assert-True (
  $blockedMutation.RetainedPaths -contains "C:\retained-install-root"
) "Unauthorized cleanup mutation lost its retained path evidence."

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

  $powerShellExecutable = Join-Path $PSHOME "pwsh.exe"
  $fixturePath = Join-Path $PSScriptRoot "windows-contained-process-fixture.ps1"
  $environmentStdout = Join-Path $testRoot "environment.stdout.log"
  $environmentStderr = Join-Path $testRoot "environment.stderr.log"
  $derivedEnvironmentValue = Join-Path $testRoot "child-value"
  [Environment]::SetEnvironmentVariable("YAP_CONTAINED_REMOVE", "parent-value", "Process")
  try {
    $environmentResult = Invoke-ContainedProcess `
      -FilePath $powerShellExecutable `
      -ArgumentList @("-NoLogo", "-NoProfile", "-NonInteractive", "-File", $fixturePath, "-Mode", "Io") `
      -TimeoutSeconds 5 `
      -StdoutPath $environmentStdout `
      -StderrPath $environmentStderr `
      -WorkingDirectory $testRoot `
      -Environment ([ordered]@{
        YAP_CONTAINED_OVERRIDE = $derivedEnvironmentValue
        YAP_CONTAINED_REMOVE = $null
      })
    Assert-True $environmentResult.CleanupProven "Environment adapter call did not prove cleanup."
    $environmentOutput = [IO.File]::ReadAllText($environmentStdout)
    Assert-True (
      $environmentOutput -match "(?m)^override=$([regex]::Escape($derivedEnvironmentValue))\r?$"
    ) "Adapter lost a provider-derived string environment override."
    Assert-True ($environmentOutput -match "(?m)^removed=\r?$") "Adapter lost an environment removal."
  } finally {
    [Environment]::SetEnvironmentVariable("YAP_CONTAINED_REMOVE", $null, "Process")
  }

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
