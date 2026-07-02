# Smoke-test the installed Yap CrispASR stack: spawn, wait for model load, transcribe one file.
param(
    [string]$AudioFile = "C:\Users\mcnatg1\OneDrive - Medtronic PLC\Desktop\00014_Wireless GO.WAV",
    [string]$YapDir = "$env:LOCALAPPDATA\Yap",
    [int]$Port = 8765,
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
$LogFile = Join-Path $LogDir "smoke-test.log"

Write-Log "=== smoke test start ==="
Write-Log "audio=$AudioFile yap_dir=$YapDir port=$Port"

if (-not (Test-Path $AudioFile)) {
    throw "Audio file not found: $AudioFile"
}

$Binary = Join-Path $YapDir "crispasr.exe"
if (-not (Test-Path $Binary)) {
    throw "crispasr.exe not found at $Binary"
}

$PinFile = Join-Path $YapDir "_up_\crispasr-version.txt"
if (-not (Test-Path $PinFile)) {
    throw "Pin file not found: $PinFile"
}

$pin = @{}
Get-Content $PinFile | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#")) { return }
    $pair = $line.Split("=", 2)
    if ($pair.Count -eq 2) { $pin[$pair[0].Trim()] = $pair[1].Trim() }
}

$Model = Join-Path $YapDir "models\$($pin['gguf_file'])"
if (-not (Test-Path $Model)) {
    $Model = Join-Path $env:LOCALAPPDATA "Yap\models\$($pin['gguf_file'])"
}
if (-not (Test-Path $Model)) {
    throw "Model not found: $($pin['gguf_file'])"
}

$modelGb = [math]::Round((Get-Item $Model).Length / 1GB, 2)
Write-Log "binary=$Binary"
Write-Log "model=$Model (${modelGb} GB)"

$stderrLog = Join-Path $LogDir "smoke-sidecar-$(Get-Date -Format 'yyyyMMdd-HHmmss').stderr.log"

Write-Log "spawning crispasr (hidden window)..."
$args = @(
    "--server",
    "--backend", "cohere",
    "-m", $Model,
    "--host", "127.0.0.1",
    "--port", "$Port",
    "-ng"
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
        if ($resp.StatusCode -eq 200 -and $resp.Content -match '"status"\s*:\s*"ok"' -and $resp.Content -match '"backend"\s*:\s*"cohere"') {
            $ready = $true
            break
        }
        if ([int]$sw.Elapsed.TotalSeconds % 10 -eq 0) {
            Write-Log "health wait $([int]$sw.Elapsed.TotalSeconds)s body=$($resp.Content)"
        }
    } catch {
        if ([int]$sw.Elapsed.TotalSeconds % 10 -eq 0) {
            Write-Log "health wait $([int]$sw.Elapsed.TotalSeconds)s (not ready yet)"
        }
    }
    Start-Sleep -Milliseconds 500
}
$loadSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)
if (-not $ready) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    throw "Sidecar not ready after ${ReadyTimeoutSec}s. See $stderrLog and $LogFile"
}
Write-Log "sidecar ready in ${loadSec}s"

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
Write-Log "=== smoke test passed ==="
Write-Log "model_load_sec=$loadSec transcribe_sec=$transcribeSec"
