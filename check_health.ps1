$outFile = "frontend_stdout.txt"
$errFile = "frontend_stderr.txt"
$process = Start-Process -FilePath "cargo" -ArgumentList "run", "--bin", "service_frontend" -PassThru -NoNewWindow -RedirectStandardOutput $outFile -RedirectStandardError $errFile
Write-Output "Started process $($process.Id). Waiting for port 3000..."

$portOpen = $false
for ($i = 0; $i -lt 60; $i++) {
    try {
        $conn = Test-NetConnection -ComputerName localhost -Port 3000 -InformationLevel Quiet
        if ($conn) {
            $portOpen = $true
            break
        }
    } catch {}
    Start-Sleep -Seconds 5
}

if (-not $portOpen) {
    Write-Output "Timeout waiting for port 3000."
} else {
    Write-Output "Port 3000 is open. Testing endpoints..."
    try {
        $response = Invoke-WebRequest -Uri "http://localhost:3000/health" -Method Get -ErrorAction Stop
        Write-Output "HEALTH: $($response.StatusCode) $($response.Content)"
    } catch {
        Write-Output "HEALTH ERROR: $_"
        if ($_.Exception.Response) { Write-Output "Status: $($_.Exception.Response.StatusCode)" }
    }

    try {
        $response = Invoke-WebRequest -Uri "http://localhost:3000/api/status" -Method Get -ErrorAction Stop
        Write-Output "STATUS: $($response.StatusCode) $($response.Content)"
    } catch {
        Write-Output "STATUS ERROR: $_"
        if ($_.Exception.Response) { Write-Output "Status: $($_.Exception.Response.StatusCode)" }
    }
}

Stop-Process -Id $process.Id -Force
Write-Output "--- STDOUT ---"
if (Test-Path $outFile) { Get-Content $outFile | Select-Object -Last 20 }
Write-Output "--- STDERR ---"
if (Test-Path $errFile) { Get-Content $errFile | Select-Object -Last 20 }
