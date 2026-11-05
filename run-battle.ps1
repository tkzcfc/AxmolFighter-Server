#!/usr/bin/env pwsh
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "battle")

if (-not (Test-Path "build")) {
    cmake -S . -B build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

cmake --build build --config Release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$exeSuffix = if ($IsWindows -or $env:OS -eq "Windows_NT") { ".exe" } else { "" }
$binDir = Join-Path $PWD "bin"
$exe = @(
    "Release/battle_server$exeSuffix",
    "Debug/battle_server$exeSuffix",
    "battle_server$exeSuffix"
) | ForEach-Object { Join-Path $binDir $_ } |
    Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1

if (-not $exe) {
    Write-Error "battle_server binary not found under $binDir"
    exit 1
}

$config = if ($args.Count -gt 0) { $args[0] } else { "config/battle.toml" }
& $exe $config @($args | Select-Object -Skip 1)
