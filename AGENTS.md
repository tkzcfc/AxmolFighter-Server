# AxmolFighter-Server

Clients connect only to the gateway.

| Service | Lang | service_id | Notes |
|---|---|---|---|
| gateway | Rust | — | Clients on :7000, internal services on :7100. Config: `game/gateway/gateway.toml` |
| game | Rust | 0 | PostgreSQL (`postgres:123456@127.0.0.1:5432/axmol_fighter`). Config: `game/game/game.toml` |
| battle | C++20 | 1 | `battle/`. Compiles the client's `mugen` with `RUNTIME_IN_AXMOL=0` |
| town | Rust | 2 | Gateway address is hardcoded |

## Commands (PowerShell; start the gateway first)
- Start: `.\run-gateway.ps1`, then `.\run-game.ps1`, `.\run-town.ps1`, `.\run-battle.ps1 [config\battle_2.toml]`
- Start or stop gateway + game + town together: `start-stack.ps1` / `start-stack-bg.ps1` / `stop-stack.ps1`. **These don't include battle.**
- Rust: in `game/`, run `cargo build` / `cargo test`. The Cargo workspace is `game/Cargo.toml` (edition 2024, Rust 1.85 or newer).
- Battle: `run-battle.ps1` runs cmake configure/build Release and then `battle/bin/Release/battle_server.exe`.

## Rust workspace (`game/`)
`protocol` (protobuf sources in `pb/*.proto`), `base`, `backend-framework` (service ids in `src/service_id.rs`), `game`, `gateway`, `town`.

## Protocol
- msg_id ranges:

  | Range | Owner |
  |---|---|
  | 1–19999 | game |
  | 20000–29999 | battle |
  | 30000–39999 | town |
  | 60000–60999 | internal game↔battle |
  | 61000–61999 | internal game↔town |

- Client frame, 11 bytes: `len:u32 cmd:u8 msg_id:u16 serial:i32`. Backend frames add `session_id:u32`, making 15 bytes.
- Serial: 0 means push, `-id` a request, `+id` a response.
- After editing `.proto` files, run `AxmolFighter-Tools/protoc/run_proto_gen.ps1`. Keep `README.md` at the repo root in sync.

## Battle server rules (`battle/src`)
- It runs on a single yasio thread. **No coroutines or concurrencpp, and no while/sleep loops**; use yasio timers.
- `framework/` holds `BackendCodec`, `GatewayClient`, `RpcManager` and `BackendSession`. `protocol/` is generated code.
- Logs go through spdlog to `logs/battle_server.log`, in English.
- It reads assets from `../AxmolFighter-Client/Content`, so the Client submodule must be present.
