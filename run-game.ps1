$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "game")
cargo run -p game -- game/game.toml @args
