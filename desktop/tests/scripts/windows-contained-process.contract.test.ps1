#requires -Version 7.4
#requires -PSEdition Core

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-True {
  param(
    [Parameter(Mandatory)]
    [bool]$Condition,

    [Parameter(Mandatory)]
    [string]$Message
  )

  if (-not $Condition) {
    throw $Message
  }
}

$root = $null
$nonce = $null
$sentinel = $null

try {
  $productionSource = Join-Path $PSScriptRoot "windows-contained-process.cs"
  $testingSource = Join-Path $PSScriptRoot "windows-contained-process.testing.cs"
  Add-Type -Path @($productionSource, $testingSource)

  $runtime = [IO.Path]::GetFullPath([Environment]::ProcessPath)
  $nonce = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(16)).ToLowerInvariant()
  $root = [IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) "yap-launch-request-$nonce"))
  New-Item -ItemType Directory -Path $root -ErrorAction Stop | Out-Null
  $sentinel = Join-Path $root ".yap-launch-request-v1"
  $sentinelStream = [IO.FileStream]::new($sentinel, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
  $sentinelStream.Dispose()
  $stdout = Join-Path $root "stdout.log"
  $stderr = Join-Path $root "stderr.log"

  $arguments = [string[]]@("", "plain", "two words", 'quote"inside', 'trail path\')
  $environment = [ordered]@{
    Path = "child-path"
    YAP_NEW = "child-new"
    YAP_REMOVE = $null
  }
  $request = [Yap.NsisSmoke.LaunchRequest]::Create(
    $runtime,
    $arguments,
    $stdout,
    $stderr,
    $root,
    $environment
  )

  Assert-True ($request.ExecutablePath -ceq $runtime) "Executable path changed."
  Assert-True ($request.Arguments.Count -eq 5) "Arguments were not retained as data."
  Assert-True ($request.EnvironmentOverrides["Path"] -ceq "child-path") "Override was lost."
  Assert-True ($request.EnvironmentRemovals -contains "YAP_REMOVE") "Removal was lost."
  $arguments[1] = "caller-mutated"
  $environment.Path = "caller-mutated"
  Assert-True ($request.Arguments[1] -ceq "plain") "Caller mutation changed request arguments."
  Assert-True ($request.EnvironmentOverrides["Path"] -ceq "child-path") "Caller mutation changed request environment."
  Assert-True (
    [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildCommandLine($request) -ceq
    ('"' + $runtime + '" "" plain "two words" "quote\"inside" "trail path\\"')
  ) "Normal argument quoting changed."

  $installDirectory = [Yap.NsisSmoke.NsisInstallDirectory]::Create("C:\Yap Test\Install")
  $nsis = [Yap.NsisSmoke.LaunchRequest]::CreateNsisInstaller(
    $runtime,
    [string[]]@("/S"),
    $installDirectory,
    $stdout,
    $stderr,
    $null,
    [ordered]@{}
  )
  Assert-True (
    [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildCommandLine($nsis) -ceq
    ('"' + $runtime + '" /S /D=C:\Yap Test\Install')
  ) "NSIS /D= was not the literal final tail."

  $entries = [string[]]@(
    '=C:=C:\work',
    'PATH=parent-path',
    'yap_remove=parent-remove',
    'ZED=last'
  )
  $block = [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildEnvironmentBlockText($request, $entries)
  Assert-True ($block.EndsWith("`0`0")) "Environment block lacks its double-NUL terminator."
  Assert-True ($block.Contains("=C:=C:\work`0")) "Hidden drive entry was lost."
  Assert-True ($block.Contains("Path=child-path`0")) "Case-insensitive override was not applied."
  Assert-True ($block.IndexOf("YAP_REMOVE=", [StringComparison]::OrdinalIgnoreCase) -lt 0) "Case-insensitive removal failed."
  Assert-True ($block.Contains("YAP_NEW=child-new`0")) "New environment entry was lost."

  $emptyEnvironmentRequest = [Yap.NsisSmoke.LaunchRequest]::Create(
    $runtime, @(),
    (Join-Path $root "empty.stdout.log"),
    (Join-Path $root "empty.stderr.log"),
    $null, [ordered]@{}
  )
  $emptyBlock = [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildEnvironmentBlockText(
    $emptyEnvironmentRequest,
    [string[]]@()
  )
  Assert-True ($emptyBlock -ceq "`0`0") "An empty environment was not double-NUL terminated."

  foreach ($operation in @(
    { [Yap.NsisSmoke.LaunchRequest]::Create("pwsh.exe", @(), $stdout, $stderr, $null, @{}) },
    { [Yap.NsisSmoke.LaunchRequest]::Create($runtime, @("bad`0arg"), $stdout, $stderr, $null, @{}) },
    { [Yap.NsisSmoke.LaunchRequest]::Create($runtime, @(), $stdout, $stdout, $null, @{}) },
    { [Yap.NsisSmoke.LaunchRequest]::Create($runtime, @(), "relative.log", $stderr, $null, @{}) },
    { [Yap.NsisSmoke.NsisInstallDirectory]::Create('C:\bad"path') },
    { [Yap.NsisSmoke.NsisInstallDirectory]::Create("C:\bad`npath") },
    { [Yap.NsisSmoke.LaunchRequest]::Create($runtime, @(), $stdout, $stderr, $null, [ordered]@{ '=C:' = 'mutate' }) },
    { [Yap.NsisSmoke.Testing.LaunchRequestProbe]::BuildEnvironmentBlockText($request, [string[]]@('Path=one', 'PATH=two')) }
  )) {
    $threw = $false
    try { & $operation } catch { $threw = $true }
    Assert-True $threw "Invalid launch input was accepted."
  }

  Write-Output "Windows contained-process request contracts passed."
}
finally {
  if ($null -ne $root -and [IO.Directory]::Exists($root)) {
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $actualRoot = [IO.Path]::GetFullPath($root).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $expectedRoot = [IO.Path]::GetFullPath((Join-Path $tempRoot "yap-launch-request-$nonce")).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $actualParent = [IO.Directory]::GetParent($actualRoot)
    $ownedPath =
      $null -ne $actualParent -and
      [StringComparer]::OrdinalIgnoreCase.Equals($actualRoot, $expectedRoot) -and
      [StringComparer]::OrdinalIgnoreCase.Equals($actualParent.FullName.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar), $tempRoot)

    $ownedSentinel = $false
    if ($ownedPath -and $null -ne $sentinel -and [IO.File]::Exists($sentinel)) {
      $expectedSentinel = Join-Path $actualRoot ".yap-launch-request-v1"
      if ([StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($sentinel), $expectedSentinel)) {
        $sentinelStream = [IO.File]::Open($sentinel, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
          $ownedSentinel = $sentinelStream.Length -eq 0
        }
        finally {
          $sentinelStream.Dispose()
        }
      }
    }

    if ($ownedPath -and $ownedSentinel) {
      Remove-Item -LiteralPath $actualRoot -Recurse -Force -ErrorAction Stop
    }
  }
}
