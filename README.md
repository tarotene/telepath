# Telepath

Schema-driven embedded RPC framework for Rust.

Telepath eliminates the dual-maintenance problem in embedded communication: the
firmware function definition is the interface definition. A single `#[command]`
attribute generates the wire shim, registers metadata, and enables dynamic
discovery — no IDL files, no manual protocol sync.

## Architecture

```mermaid
sequenceDiagram
    participant H as Host (telepath-host)
    participant T as Target Firmware (telepath-firmware)

    Note over T: Link time: #[command] collects CommandMetadata
    H->>T: Connect (transport open)
    H->>T: Request [CmdID: 0x0000] — Discovery
    T-->>H: Response — command list {id, name}

    Note over H: Build dynamic client map

    H->>T: Request [CmdID: N] — postcard-encoded args
    T->>T: Deserialize args → dispatch → serialize result
    T-->>H: Response [status, postcard-encoded payload]
    H->>H: Decode payload → present to caller
```

### Workspace structure

| Path | Role |
|------|------|
| `crates/telepath-wire` | Shared wire types — `no_std`, no alloc |
| `crates/telepath-macros` | `#[command]` proc-macro |
| `crates/telepath-firmware` | Target-side RPC server — `no_std` |
| `crates/telepath-host` | Host-side RPC client — `std` |
| `examples/host-emulator` | In-process server+client emulator — no hardware required |
| `examples/nrf52840-dk` | Standalone firmware example (workspace-excluded) |
| `tools/telepath-cli` | Host-side CLI over RTT (workspace-excluded) |

### Framing

| Direction | Method | Rationale |
|-----------|--------|-----------|
| Host → Target | COBS | Minimal decoder on MCU: `read_until(0x00)` |
| Target → Host | COBS (rzCOBS planned) | rzCOBS improves throughput for sparse sensor data — see [C2 in the MVP roadmap](https://github.com/tarotene/telepath/issues/3) |

Both directions use `0x00` as the frame delimiter.

### Packet model

Two packet types only (`Request` / `Response`), following the ONC RPC RFC 5531
CALL/REPLY model. Errors live in `ResponseStatus`, not as separate packet types.
CmdID `0x0000` is reserved for the Command Discovery Protocol (CDP).

## Quickstart

The fastest way to see Telepath in action requires no hardware.

```
git clone https://github.com/tarotene/telepath.git
cd telepath
cargo run -p host-emulator
```

Expected output:

```
ping -> 0xDEADBEEF
```

The emulator runs a `TelepathServer` and `TelepathClient` on two OS threads
connected by `std::sync::mpsc` byte channels. The full wire path
(postcard serialization + COBS framing) executes identically to real hardware.
Switching to an MCU is purely a transport swap.

## Prerequisites

| Tool | Purpose |
|------|---------|
| Rust stable | Build host workspace |
| `rustup target add thumbv7em-none-eabi` | Firmware cross-compilation |
| `probe-rs` | Flash and run firmware on nRF52840-DK |
| `just` | Task runner (optional but recommended) |

## Build

```
# Host workspace
cargo build --workspace

# Run host tests
cargo test --workspace

# Firmware example — must cd so .cargo/config.toml is discovered
cd examples/nrf52840-dk && cargo build --release

# Flash to hardware (downloads and exits; probe is released immediately)
cd examples/nrf52840-dk && cargo run --release

# CLI tool — must cd because it is workspace-excluded
cd tools/telepath-cli && cargo build

# 1-shot ping (firmware must already be flashed)
cd tools/telepath-cli && cargo run -- ping

# Interactive REPL
cd tools/telepath-cli && cargo run
```

## Real hardware: nRF52840-DK

> **Experimental — not yet verified end-to-end.** The wire layer is exercised
> by `host-emulator` (CI smoke test), but the RTT-backed hardware path has not
> been re-verified since the CLI refactor in PR #6 and currently returns no
> response on `ping`. Tracked in [#9](https://github.com/tarotene/telepath/issues/9).
> Use the hardware-free [Quickstart](#quickstart) for a working demo.

See [`examples/nrf52840-dk/README.md`](examples/nrf52840-dk/README.md) for the
full hardware walk-through (udev rules, APPROTECT unlock, RTT channel layout).

```
# Flash firmware (downloads and exits; probe is released)
cd examples/nrf52840-dk && cargo run --release

# Ping over RTT — currently times out; see issue #9
cd tools/telepath-cli && cargo run -- ping
```

## Using telepath as a library

### Firmware side

```toml
# Cargo.toml
[dependencies]
telepath-firmware = { git = "https://github.com/tarotene/telepath", branch = "main" }
postcard          = { version = "1", default-features = false }
```

```rust
use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};
use telepath_firmware::transport::Transport;

// 1. Define a shim by hand (until `#[command]` is fully implemented).
fn ping_shim(_input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError> {
    let v: u32 = 0xDEAD_BEEF;
    Ok(postcard::to_slice(&v, output).map_err(|_| DispatchError::SerializeError)?.len())
}

static COMMANDS: [CommandMetadata; 1] = [CommandMetadata {
    name: "ping", id: 0x0001, invoke: ping_shim,
}];

// 2. Implement `Transport` for your byte-stream peripheral (UART, RTT, USB …).
//    Non-blocking: `fn read(&mut self, &mut [u8]) -> usize` / `fn write(&mut self, &[u8]) -> usize`.

let mut server = TelepathServer::<MyTransport, 512>::new(transport, &COMMANDS);
loop { server.poll(); }
```

> ⚠️ **`#[command]` is currently a passthrough stub.** Code generation for shims
> and metadata registration is planned for Stage B of the
> [MVP roadmap](https://github.com/tarotene/telepath/issues/3). Until then,
> define `CommandMetadata` and the shim function manually as shown above.

### Host side

```toml
[dependencies]
telepath-host = { git = "https://github.com/tarotene/telepath", branch = "main" }
postcard      = "1"
```

```rust
use telepath_host::TelepathClient;

// transport: anything implementing `std::io::Read + std::io::Write`
let mut client = TelepathClient::new(transport);
let payload = client.call_raw(0x0001, &[])?;
let val: u32 = postcard::from_bytes(&payload)?;
println!("ping -> 0x{:08X}", val);
```

## CI / Quality gates

```
# Format check
cargo fmt --all -- --check

# Clippy (warnings are errors)
cargo clippy --workspace -- -D warnings

# All checks at once
just ci
```

## License

Licensed under either of

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.
