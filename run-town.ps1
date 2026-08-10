$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "game")
cargo run -p town @args
