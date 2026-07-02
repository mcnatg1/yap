# Benchmark one CrispASR backend on the standard smoke audio file.
param(
    [Parameter(Mandatory = $true)]
    [string]$Backend,
    [Parameter(Mandatory = $true)]
    [string]$ModelPath,
    [string]$AudioFile = "C:\Users\mcnatg1\OneDrive - Medtronic PLC\Desktop\00014_Wireless GO.WAV",
    [string]$YapDir = "$env:LOCALAPPDATA\Yap",
    [int]$Port = 8777,
    [int]$ReadyTimeoutSec = 300
)

$ErrorActionPreference = "Stop"

function Write-Log {
    param([string]$Message)
    $line = "$(Get-Date -Format o) $Message"
    Write-Host $line
    Add-Content -Path $LogFile -Value $line
}

$LogDir = Join-Path $YapDir "logs"
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$LogFile = Join-Path $LogDir "benchmark-$Backend.log"

Write-Log "=== benchmark start backend=$Backend port=$Port ==="
Write-Log "audio=$AudioFile"

if (-not (Test-Path $AudioFile)) { throw "Audio file not found: $AudioFile" }
if (-not (Test-Path $ModelPath)) { throw "Model not found: $ModelPath" }

$Binary = Join-Path $YapDir "crispasr.exe"
if (-not (Test-Path $Binary)) { throw "crispasr.exe not found at $Binary" }

Get-Process crispasr -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1

$modelMb = [math]::Round((Get-Item $ModelPath).Length / 1MB, 1)
Write-Log "binary=$Binary"
Write-Log "model=$ModelPath (${modelMb} MB)"

$stderrLog = Join-Path $LogDir "benchmark-$Backend-$(Get-Date -Format 'yyyyMMdd-HHmmss').stderr.log"
$args = @(
    "--server",
    "--backend", $Backend,
    "-m", $ModelPath,
    "--host", "127.0.0.1",
    "--port", "$Port",
    "-ng",
    "-l", "en"
)
$proc = Start-Process -FilePath $Binary -ArgumentList $args -PassThru `
    -WindowStyle Hidden -RedirectStandardError $stderrLog

$healthUrl = "http://127.0.0.1:$Port/health"
$ready = $false
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt $ReadyTimeoutSec) {
    if ($proc.HasExited) {
        throw "crispasr exited early with code $($proc.ExitCode). See $stderrLog"
    }
    try {
        $resp = Invoke-WebRequest -Uri $healthUrl -UseBasicParsing -TimeoutSec 2
        if ($resp.StatusCode -eq 200 -and $resp.Content -match '"status"\s*:\s*"ok"') {
            $ready = $true
            break
        }
    } catch {}
    Start-Sleep -Milliseconds 500
}
$loadSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)
if (-not $ready) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    throw "Sidecar not ready after ${ReadyTimeoutSec}s. See $stderrLog"
}
Write-Log "sidecar ready in ${loadSec}s health=$(Invoke-WebRequest -Uri $healthUrl -UseBasicParsing).Content"

Write-Log "transcribing..."
$transcribeSw = [System.Diagnostics.Stopwatch]::StartNew()
$transcribeUrl = "http://127.0.0.1:$Port/v1/audio/transcriptions"
$responseJson = curl.exe -s -X POST $transcribeUrl -F "file=@$AudioFile" -F "language=en"
if ($LASTEXITCODE -ne 0 -or -not $responseJson) {
    throw "Transcription request failed. curl exit=$LASTEXITCODE"
}
$response = $responseJson | ConvertFrom-Json
$transcribeSec = [math]::Round($transcribeSw.Elapsed.TotalSeconds, 1)
$textPreview = if ($response.text.Length -gt 120) { $response.text.Substring(0, 120) + "..." } else { $response.text }
Write-Log "transcribe complete in ${transcribeSec}s chars=$($response.text.Length) preview=$textPreview"

Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Write-Log "=== benchmark done backend=$Backend load_sec=$loadSec transcribe_sec=$transcribeSec ==="

Write-Output @{
    backend = $Backend
    model_load_sec = $loadSec
    transcribe_sec = $transcribeSec
    chars = $response.text.Length
} | ConvertTo-Json
