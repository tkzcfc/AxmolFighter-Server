$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "game")
cargo run -p town -- town/town.toml @args
