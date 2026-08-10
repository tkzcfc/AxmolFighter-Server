$ErrorActionPreference = "Continue"
$pidFile = Join-Path $PSScriptRoot ".run-stack.pids"

if (Test-Path $pidFile) {
    $pids = Get-Content $pidFile | Where-Object { $_ -match '^\d+$' }
    foreach ($procId in $pids) {
        Write-Host "Stopping process tree PID $procId..."
        & taskkill /PID $procId /T /F 2>$null | Out-Null
    }
    Remove-Item $pidFile -Force -ErrorAction SilentlyContinue
}

foreach ($name in @("gateway", "game", "town")) {
    Get-Process -Name $name -ErrorAction SilentlyContinue | ForEach-Object {
        Write-Host "Stopping leftover $($_.Name) (PID $($_.Id))..."
        Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "Stack stopped."
