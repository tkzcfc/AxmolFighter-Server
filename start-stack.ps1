$ErrorActionPreference = "Stop"

$gameDir = Join-Path $PSScriptRoot "game"
$pidFile = Join-Path $PSScriptRoot ".run-stack.pids"
$originalLocation = Get-Location

try {
    Set-Location $gameDir

    Write-Host "Building gateway, game, town..."
    cargo build -p gateway -p game -p town
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    function Start-ServerWindow {
        param(
            [string]$Title,
            [string]$CargoArgs
        )

        $command = @"
`$Host.UI.RawUI.WindowTitle = '$Title'
Set-Location '$gameDir'
Write-Host '=== $Title ===' -ForegroundColor Cyan
cargo $CargoArgs
Write-Host ''
Write-Host 'Process exited. Close this window or press Enter.' -ForegroundColor Yellow
Read-Host
"@

        Start-Process -FilePath "powershell" -ArgumentList @(
            "-NoExit",
            "-NoProfile",
            "-Command",
            $command
        ) -PassThru
    }

    $pids = @()

    Write-Host "Starting gateway (new window)..."
    $gw = Start-ServerWindow -Title "gateway" -CargoArgs "run -p gateway -- gateway/gateway.toml"
    $pids += $gw.Id
    Start-Sleep -Seconds 2

    Write-Host "Starting game (new window)..."
    $game = Start-ServerWindow -Title "game" -CargoArgs "run -p game -- game/game.toml"
    $pids += $game.Id
    Start-Sleep -Seconds 1

    Write-Host "Starting town (new window)..."
    $town = Start-ServerWindow -Title "town" -CargoArgs "run -p town"
    $pids += $town.Id

    $pids | Set-Content -Path $pidFile -Encoding ascii
    Write-Host "Stack started in 3 foreground windows. PIDs: $($pids -join ', ')"
    Write-Host "Stop with: .\stop-stack.ps1"
}
finally {
    Set-Location $originalLocation
}
