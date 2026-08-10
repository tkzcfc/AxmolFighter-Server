$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "battle")

if (-not (Test-Path "build")) {
    cmake -S . -B build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

cmake --build build --config Release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$exe = @(
    "bin\Release\battle_server.exe",
    "bin\Debug\battle_server.exe",
    "bin\battle_server.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $exe) {
    Write-Error "battle_server.exe not found under bin/"
    exit 1
}

$config = if ($args.Count -gt 0) { $args[0] } else { "config/battle.toml" }
& $exe $config @($args | Select-Object -Skip 1)
