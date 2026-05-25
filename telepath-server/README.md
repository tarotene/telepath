# telepath-server

RPC **server** library for embedded targets (`no_std`, no alloc).

Target-side Telepath RPC server. Runs on the MCU in `#![no_std]` mode.

Receives COBS-framed postcard requests from the host, dispatches them to
registered command handlers, and sends COBS-framed postcard responses back.

## Architecture

```mermaid
flowchart LR
    T[Transport] -->|bytes| FA[FrameAccumulator]
    FA -->|COBS frame| CD[cobs_decode]
    CD -->|raw bytes| PD["postcard::from_bytes"]
    PD -->|typed args| D[dispatch]
    D -->|result| PS["postcard::to_slice"]
    PS -->|raw bytes| CE[cobs_encode]
    CE -->|COBS frame| T
```

## Key API

### `TelepathServer<T, N>`

| Method | Description |
|--------|-------------|
| `new(transport, commands)` | Create a server with a transport and a static command slice |
| `poll()` | Drain available bytes, process any complete frames; call in a loop |
| `dispatch(cmd_id, input, output)` | Manually dispatch a decoded payload (useful for testing) |
| `find_command(id)` | Look up a command by ID (linear scan over the static slice) |

`T` must implement `transport::Transport`. `N` is the internal buffer
size; use `512` or larger to accommodate max-payload frames.

### `transport::Transport` trait

```rust
pub trait Transport {
    fn read(&mut self, buf: &mut [u8]) -> usize;
    fn write(&mut self, buf: &[u8]) -> usize;
}
```

Both methods are non-blocking and return the number of bytes transferred.

### `CommandMetadata`

```rust
pub struct CommandMetadata {
    pub name: &'static str,
    pub id: u16,
    pub invoke: ShimFn,   // fn(&[u8], &mut [u8]) -> Result<usize, DispatchError>
}
```

Register commands by passing a `&'static [CommandMetadata]` to `new()`.
Use `telepath_server::commands()` (linkme-collected at link time) to
pass all `#[command]`-annotated functions automatically.

## Usage

```rust
use telepath_server::{command, TelepathServer};

#[command]
fn ping() -> u32 { 0xDEAD_BEEF }

// Peripheral state can be injected type-safely with #[resource].
// Resource args are not serialized; they are provided by the server's ResourceRegistry.
#[command]
fn set_led(#[resource] led: &mut MyLed, on: bool) -> bool { led.set(on) }

let mut server = TelepathServer::<MyTransport, 512>::new(
    transport,
    telepath_server::commands(),
);
loop {
    server.poll();
}
```

> **Note:** The `#[command]` macro generates a shim that calls
> `postcard::from_bytes` / `postcard::to_slice` directly. Your firmware
> crate must add `postcard` as a direct dependency in its `Cargo.toml`
> in addition to `telepath-server`.

## Build

This crate targets both native (for tests) and `thumbv7em-none-eabi` (for firmware).

```
# Host tests
cargo test -p telepath-server

# Cross-compiled (requires the target to be added)
rustup target add thumbv7em-none-eabi
cargo build -p telepath-server --target thumbv7em-none-eabi
```

## Limitations

- Upstream (target → host) framing uses COBS in this MVP. rzCOBS is planned
  for Stage C (see [#76](https://github.com/tarotene/telepath/issues/76)).

