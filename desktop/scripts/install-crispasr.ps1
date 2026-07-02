# Downloads the pinned CrispASR release from GitHub, verifies SHA-256, and installs.
# Used by the NSIS installer (next to Yap.exe) and by dev fetch (LOCALAPPDATA cache).
param(
    [string]$InstallDir,
    [switch]$UseCacheDir,
    [switch]$IfNeeded,
    [string]$PinFile
)

$ErrorActionPreference = "Stop"

function Read-Pin {
    param([string]$Path)
    if (-not (Test-Path $Path)) {
        throw "Pin file not found: $Path"
    }
    $values = @{}
    Get-Content $Path | ForEach-Object {
        $line = $_.Trim()
        if (-not $line -or $line.StartsWith("#")) { return }
        $pair = $line.Split("=", 2)
        if ($pair.Count -eq 2) {
            $values[$pair[0].Trim()] = $pair[1].Trim()
        }
    }
    return $values
}

function Resolve-PinFile {
    param(
        [string]$ExplicitPin,
        [string]$InstallDirectory,
        [string]$ScriptRoot
    )

    if ($ExplicitPin) {
        return (Resolve-Path $ExplicitPin).Path
    }

    $candidates = @(
        (Join-Path $ScriptRoot "crispasr-version.txt")
    )
    if ($InstallDirectory) {
        $candidates += @(
            (Join-Path $InstallDirectory "_up_\crispasr-version.txt"),
            (Join-Path $InstallDirectory "resources\crispasr-version.txt"),
            (Join-Path $InstallDirectory "crispasr-version.txt")
        )
    }

    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            return (Resolve-Path $candidate).Path
        }
    }

    throw "Could not locate crispasr-version.txt"
}

function Resolve-Destination {
    param(
        [string]$Version,
        [string]$InstallDirectory,
        [switch]$CacheDir
    )

    if ($CacheDir) {
        $root = if ($env:LOCALAPPDATA) {
            Join-Path $env:LOCALAPPDATA "Yap\bin"
        } else {
            Join-Path $env:TEMP "Yap\bin"
        }
        New-Item -ItemType Directory -Force -Path $root | Out-Null
        return Join-Path $root "crispasr-$Version.exe"
    }

    if (-not $InstallDirectory) {
        throw "InstallDir is required unless -UseCacheDir is set"
    }

    return Join-Path $InstallDirectory "crispasr.exe"
}

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$resolvedPin = Resolve-PinFile -ExplicitPin $PinFile -InstallDirectory $InstallDir -ScriptRoot $scriptRoot
$pin = Read-Pin $resolvedPin
$version = $pin["crispasr_version"]
$expectedHash = $pin["binary_sha256"].ToLower()
$asset = "crispasr-windows-x86_64-cpu.zip"
$member = "crispasr-windows-x86_64-cpu/crispasr.exe"
$url = "https://github.com/CrispStrobe/CrispASR/releases/download/v$version/$asset"

if (-not $version -or -not $expectedHash) {
    throw "crispasr-version.txt is missing crispasr_version or binary_sha256"
}

$dest = Resolve-Destination -Version $version -InstallDirectory $InstallDir -CacheDir:$UseCacheDir

if ($IfNeeded -and (Test-Path $dest)) {
    $actual = (Get-FileHash -Algorithm SHA256 $dest).Hash.ToLower()
    if ($actual -eq $expectedHash) {
        Write-Host "CrispASR $version already present and verified at $dest"
        exit 0
    }
    Write-Host "Existing binary failed SHA-256 check; re-downloading..."
}

$work = Join-Path $env:TEMP "yap-install-crispasr-$version"
$zip = Join-Path $work $asset
$extract = Join-Path $work "extract"

Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $work, $extract | Out-Null
if ($InstallDir -and -not $UseCacheDir) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

Write-Host "Downloading CrispASR v$version from $url ..."
Invoke-WebRequest -Uri $url -OutFile $zip
Expand-Archive -Path $zip -DestinationPath $extract -Force

$source = Join-Path $extract $member
if (-not (Test-Path $source)) {
    throw "Archive missing expected member: $member"
}

$hash = (Get-FileHash -Algorithm SHA256 $source).Hash.ToLower()
if ($hash -ne $expectedHash) {
    throw "SHA-256 mismatch for CrispASR v$version. Expected $expectedHash but got $hash"
}

Copy-Item -Force $source $dest
Write-Host "Installed verified CrispASR v$version to $dest"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
