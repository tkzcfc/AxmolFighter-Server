#!/usr/bin/env pwsh
$ErrorActionPreference = "Stop"

$gameDir = Join-Path $PSScriptRoot "game"
$pidFile = Join-Path $PSScriptRoot ".run-stack.pids"
$binDir = Join-Path (Join-Path $gameDir "target") "debug"
$exeSuffix = if ($IsWindows -or $env:OS -eq "Windows_NT") { ".exe" } else { "" }
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

        if ($IsMacOS) {
            # Open a new Terminal window that runs the server in the foreground.
            $quotedArgs = ($ExeArgs | ForEach-Object { "'$_'" }) -join " "
            $shellCmd = "printf '\033]0;$Title\007'; cd '$gameDir' && echo '=== $Title ===' && '$ExePath' $quotedArgs; echo; echo 'Process exited. Close this window.'"
            $escaped = $shellCmd.Replace('\', '\\').Replace('"', '\"')
            & osascript -e "tell application `"Terminal`" to do script `"$escaped`"" -e 'tell application "Terminal" to activate' | Out-Null
            return
        }

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

        $proc = Start-Process -FilePath "powershell" -ArgumentList @(
            "-NoExit",
            "-NoProfile",
            "-Command",
            $command
        ) -PassThru
        return $proc.Id
    }

    $gatewayExe = Join-Path $binDir "gateway$exeSuffix"
    $gameExe = Join-Path $binDir "game$exeSuffix"
    $townExe = Join-Path $binDir "town$exeSuffix"

    foreach ($exe in @($gatewayExe, $gameExe, $townExe)) {
        if (-not (Test-Path $exe)) {
            Write-Error "Built binary not found: $exe"
            exit 1
        }
    }

    $pids = @()

    Write-Host "Starting gateway (new window)..."
    $pids += Start-ServerWindow -Title "gateway" -ExePath $gatewayExe -ExeArgs @("gateway/gateway.toml")
    Start-Sleep -Seconds 2

    Write-Host "Starting game (new window)..."
    $pids += Start-ServerWindow -Title "game" -ExePath $gameExe -ExeArgs @("game/game.toml")
    Start-Sleep -Seconds 1

    Write-Host "Starting town (new window)..."
    $pids += Start-ServerWindow -Title "town" -ExePath $townExe

    # Windows PowerShell 5 has no $IsWindows; detect via OS env as well.
    $onWindows = ($IsWindows -eq $true) -or ($env:OS -eq "Windows_NT")
    if (-not $onWindows) {
        # Terminal windows don't expose the server PID; look the processes up by binary path.
        Start-Sleep -Seconds 1
        $pids = @($gatewayExe, $gameExe, $townExe) | ForEach-Object { & pgrep -f "^$_" } | Where-Object { $_ }
    }
    $pids = @($pids | Where-Object { $_ })

    $pids | Set-Content -Path $pidFile -Encoding ascii
    Write-Host "Stack started in 3 foreground windows. PIDs: $($pids -join ', ')"
    Write-Host "Stop with: ./stop-stack.ps1"
}
finally {
    Set-Location $originalLocation
}
