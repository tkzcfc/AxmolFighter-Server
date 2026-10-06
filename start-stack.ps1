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

    function Start-ServerWindow {
        param(
            [string]$Title,
            [string]$ExePath,
            [string[]]$ExeArgs = @()
        )

        $argList = ($ExeArgs | ForEach-Object { "'$_'" }) -join ", "
        $command = @"
`$Host.UI.RawUI.WindowTitle = '$Title'
Set-Location '$gameDir'
Write-Host '=== $Title ===' -ForegroundColor Cyan
& '$ExePath' @($argList)
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

    Write-Host "Starting gateway (new window)..."
    $gw = Start-ServerWindow -Title "gateway" -ExePath $gatewayExe -ExeArgs @("gateway/gateway.toml")
    $pids += $gw.Id
    Start-Sleep -Seconds 2

    Write-Host "Starting game (new window)..."
    $game = Start-ServerWindow -Title "game" -ExePath $gameExe -ExeArgs @("game/game.toml")
    $pids += $game.Id
    Start-Sleep -Seconds 1

    Write-Host "Starting town (new window)..."
    $town = Start-ServerWindow -Title "town" -ExePath $townExe
    $pids += $town.Id

    $pids | Set-Content -Path $pidFile -Encoding ascii
    Write-Host "Stack started in 3 foreground windows. PIDs: $($pids -join ', ')"
    Write-Host "Stop with: .\stop-stack.ps1"
}
finally {
    Set-Location $originalLocation
}
