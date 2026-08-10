$ErrorActionPreference = "Stop"

$gameDir = Join-Path $PSScriptRoot "game"
$pidFile = Join-Path $PSScriptRoot ".run-stack.pids"
$originalLocation = Get-Location

try {
    Set-Location $gameDir

    Write-Host "Building gateway, game, town..."
    cargo build -p gateway -p game -p town
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $pids = @()

    Write-Host "Starting gateway..."
    $gw = Start-Process -FilePath "cargo" -ArgumentList @("run", "-p", "gateway", "--", "gateway/gateway.toml") `
        -WorkingDirectory $gameDir -PassThru
    $pids += $gw.Id
    Start-Sleep -Seconds 2

    Write-Host "Starting game..."
    $game = Start-Process -FilePath "cargo" -ArgumentList @("run", "-p", "game", "--", "game/game.toml") `
        -WorkingDirectory $gameDir -PassThru
    $pids += $game.Id
    Start-Sleep -Seconds 1

    Write-Host "Starting town..."
    $town = Start-Process -FilePath "cargo" -ArgumentList @("run", "-p", "town") `
        -WorkingDirectory $gameDir -PassThru
    $pids += $town.Id

    $pids | Set-Content -Path $pidFile -Encoding ascii
    Write-Host "Stack started (background). PIDs: $($pids -join ', ') (saved to .run-stack.pids)"
    Write-Host "Stop with: .\stop-stack.ps1"
}
finally {
    Set-Location $originalLocation
}
