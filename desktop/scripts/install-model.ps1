# Downloads the pinned GGUF model from Hugging Face, verifies SHA-256, and caches it.
# Used by the NSIS installer and by dev fetch (%LOCALAPPDATA%\Yap\models\).
param(
    [string]$InstallDir,
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

function Resolve-ModelsDir {
    if ($env:YAP_MODELS_DIR) {
        return $env:YAP_MODELS_DIR
    }
    if ($env:LOCALAPPDATA) {
        return Join-Path $env:LOCALAPPDATA "Yap\models"
    }
    return Join-Path (Get-Location) "models"
}

function Get-VerifiedMarkerPath {
    param([string]$ModelPath)
    return [System.IO.Path]::ChangeExtension($ModelPath, "verified")
}

function Test-TrustedCache {
    param(
        [string]$ModelPath,
        [string]$ExpectedHash
    )

    $marker = Get-VerifiedMarkerPath $ModelPath
    if (-not (Test-Path $ModelPath) -or -not (Test-Path $marker)) {
        return $false
    }

    $lines = Get-Content $marker
    if ($lines.Count -lt 2) {
        return $false
    }

    $hash = $lines[0].Trim().ToLower()
    $size = [int64]$lines[1]
    $actualSize = (Get-Item $ModelPath).Length
    return ($hash -eq $ExpectedHash.ToLower()) -and ($size -eq $actualSize)
}

function Write-VerifiedMarker {
    param(
        [string]$ModelPath,
        [string]$ExpectedHash
    )

    $size = (Get-Item $ModelPath).Length
    $marker = Get-VerifiedMarkerPath $ModelPath
    [System.IO.File]::WriteAllText($marker, "$ExpectedHash`n$size`n")
}

function Get-HfResolveUrl {
    param(
        [string]$Repo,
        [string]$Revision,
        [string]$File
    )
    return "https://huggingface.co/$Repo/resolve/$Revision/$File"
}

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$resolvedPin = Resolve-PinFile -ExplicitPin $PinFile -InstallDirectory $InstallDir -ScriptRoot $scriptRoot
$pin = Read-Pin $resolvedPin

$repo = $pin["gguf_repo"]
$revision = $pin["gguf_revision"]
$file = $pin["gguf_file"]
$expectedHash = $pin["gguf_sha256"].ToLower()

if (-not $repo -or -not $revision -or -not $file -or -not $expectedHash) {
    throw "crispasr-version.txt is missing gguf_repo, gguf_revision, gguf_file, or gguf_sha256"
}

$modelsDir = Resolve-ModelsDir
New-Item -ItemType Directory -Force -Path $modelsDir | Out-Null
$dest = Join-Path $modelsDir $file

if ($IfNeeded) {
    if (Test-TrustedCache -ModelPath $dest -ExpectedHash $expectedHash) {
        Write-Host "Model already present and verified at $dest"
        exit 0
    }
    if (Test-Path $dest) {
        $actual = (Get-FileHash -Algorithm SHA256 $dest).Hash.ToLower()
        if ($actual -eq $expectedHash) {
            Write-VerifiedMarker -ModelPath $dest -ExpectedHash $expectedHash
            Write-Host "Model already present and verified at $dest"
            exit 0
        }
        Write-Host "Existing model failed SHA-256 check; re-downloading..."
        Remove-Item -Force $dest -ErrorAction SilentlyContinue
        Remove-Item -Force (Get-VerifiedMarkerPath $dest) -ErrorAction SilentlyContinue
    }
}

$url = Get-HfResolveUrl -Repo $repo -Revision $revision -File $file
$tmp = "$dest.part"

Write-Host "Downloading transcription model from $url ..."
Write-Host "Destination: $dest"
Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing

$hash = (Get-FileHash -Algorithm SHA256 $tmp).Hash.ToLower()
if ($hash -ne $expectedHash) {
    Remove-Item -Force $tmp -ErrorAction SilentlyContinue
    throw "SHA-256 mismatch for $file. Expected $expectedHash but got $hash"
}

Move-Item -Force $tmp $dest
Write-VerifiedMarker -ModelPath $dest -ExpectedHash $expectedHash
Write-Host "Installed verified model to $dest"
