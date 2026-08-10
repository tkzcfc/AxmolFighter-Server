$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "game")
cargo run -p gateway -- gateway/gateway.toml @args
