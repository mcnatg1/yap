# Dev helper: download verified CrispASR + GGUF model into %LOCALAPPDATA%\Yap\
param(
    [switch]$IfNeeded
)

$ErrorActionPreference = "Stop"
$installScript = Join-Path $PSScriptRoot "install-crispasr.ps1"
$modelScript = Join-Path $PSScriptRoot "install-model.ps1"
$desktopRoot = Split-Path -Parent $PSScriptRoot
$pinFile = Join-Path $desktopRoot "crispasr-version.txt"

& $installScript -UseCacheDir -PinFile $pinFile @PSBoundParameters
& $modelScript -PinFile $pinFile @PSBoundParameters
