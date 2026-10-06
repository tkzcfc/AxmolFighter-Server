$ErrorActionPreference = "Stop"

$gameDir = Join-Path $PSScriptRoot "game"
$pidFile = Join-Path $PSScriptRoot ".run-stack.pids"
$binDir = Join-Path $gameDir "target\debug"
$originalLocation = Get-Location

try {
    Set-Location $gameDir

    Write-Host "Building gateway, game, town..."
    cargo build -p gateway -p game -p town
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $gatewayExe = Join-Path $binDir "gateway.exe"
    $gameExe = Join-Path $binDir "game.exe"
    $townExe = Join-Path $binDir "town.exe"

    foreach ($exe in @($gatewayExe, $gameExe, $townExe)) {
        if (-not (Test-Path $exe)) {
            Write-Error "Built binary not found: $exe"
            exit 1
        }
    }

    $pids = @()

    Write-Host "Starting gateway..."
    $gw = Start-Process -FilePath $gatewayExe -ArgumentList @("gateway/gateway.toml") `
        -WorkingDirectory $gameDir -PassThru
    $pids += $gw.Id
    Start-Sleep -Seconds 2

    Write-Host "Starting game..."
    $game = Start-Process -FilePath $gameExe -ArgumentList @("game/game.toml") `
        -WorkingDirectory $gameDir -PassThru
    $pids += $game.Id
    Start-Sleep -Seconds 1

    Write-Host "Starting town..."
    $town = Start-Process -FilePath $townExe -WorkingDirectory $gameDir -PassThru
    $pids += $town.Id

    $pids | Set-Content -Path $pidFile -Encoding ascii
    Write-Host "Stack started (background). PIDs: $($pids -join ', ') (saved to .run-stack.pids)"
    Write-Host "Stop with: .\stop-stack.ps1"
}
finally {
    Set-Location $originalLocation
}
