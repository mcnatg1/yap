param(
    [string]$InstallDir,
    [switch]$IfNeeded,
    [string]$PinFile
)

$ErrorActionPreference = "Stop"
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path

& (Join-Path $scriptRoot "install-crispasr.ps1") -InstallDir $InstallDir -IfNeeded:$IfNeeded -PinFile $PinFile
& (Join-Path $scriptRoot "install-model.ps1") -InstallDir $InstallDir -IfNeeded:$IfNeeded -PinFile $PinFile
