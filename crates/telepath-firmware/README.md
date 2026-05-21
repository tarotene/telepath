# telepath-firmware

Target-side Telepath RPC server. Runs on the MCU in `#![no_std]` mode.

Receives COBS-framed postcard requests from the host, dispatches them to
registered command handlers, and sends COBS-framed postcard responses back.

## Architecture

```
Transport
  └─► FrameAccumulator ─► cobs_decode ─► postcard::from_bytes ─► dispatch()
                                                                       │
Transport ◄── cobs_encode ◄── postcard::to_slice ◄──────────────── handler
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
Use `telepath_firmware::commands()` (linkme-collected at link time) to
pass all `#[command]`-annotated functions automatically.

## Usage

```rust
use telepath_firmware::{command, TelepathServer};

#[command]
fn ping() -> u32 { 0xDEAD_BEEF }

let mut server = TelepathServer::<MyTransport, 512>::new(
    transport,
    telepath_firmware::commands(),
);
loop {
    server.poll();
}
```

## Build

This crate targets both native (for tests) and `thumbv7em-none-eabi` (for firmware).

```
# Host tests
cargo test -p telepath-firmware

# Cross-compiled (requires the target to be added)
rustup target add thumbv7em-none-eabi
cargo build -p telepath-firmware --target thumbv7em-none-eabi
```

## Limitations

- `handle_discovery` (CmdID `0x0000`) returns the registered command list as a
  postcard sequence of `(id, name)` pairs. Schema fingerprints (B4b) and
  chunked responses for command lists exceeding 256 bytes (B4c) are deferred —
  see [roadmap](https://github.com/tarotene/telepath/issues/3).
- Upstream (target → host) framing uses COBS in this MVP. rzCOBS is planned
  for Stage C (roadmap C2).
- No typed `call::<Args, Ret>` on the firmware side yet (roadmap C1).
