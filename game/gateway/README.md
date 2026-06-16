# gateway

A minimal gateway server bootstrap built on top of the `base` networking module.

## Implemented

- Command-line listener configuration per protocol and port
- Multiple listeners started in the same process
- Optional TLS via `--tls-cert` + `--tls-key`
- Graceful shutdown on `Ctrl+C`
- A welcome message is sent as soon as a client connects
- Client payloads are echoed back for basic connectivity testing

## Supported protocols

- `tcp`
- `ws`
- `kcp`

## Listener syntax

Each listener can be passed in one of these forms:

- `--listen tcp=1000`
- `--listen ws:2000`
- `--listen kcp=3000`

You can repeat `--listen` to start multiple listeners:

```powershell
cargo run -p gateway -- --listen tcp=1000 --listen ws=2000 --listen kcp=3000
```

Bind to a specific host:

```powershell
cargo run -p gateway -- --host 127.0.0.1 --listen tcp=1000 --listen ws=2000 --listen kcp=3000
```

Enable TLS:

```powershell
cargo run -p gateway -- --listen tcp=1000 --listen ws=2000 --listen kcp=3000 --tls-cert .\certs\server.pem --tls-key .\certs\server.key
```

## TLS behavior

- `tcp` supports TLS
- `ws` becomes `wss` when TLS is enabled
- `kcp` also uses the TLS path exposed by the `base` module

## Connection behavior

When a client connects, the gateway sends a welcome payload like this:

```json
{"type":"welcome","server":"gateway","protocol":"tcp","bind":"0.0.0.0:1000","tls":false,"session_id":1,"peer":"127.0.0.1:54321"}
```

After that, every message sent by the client is echoed back unchanged. This is intended as a simple first-step connectivity check before adding real gateway routing logic.
