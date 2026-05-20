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
The `#[command]` attribute macro (currently a passthrough stub) is
intended to generate these entries automatically.

## Usage

```rust
use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};

fn ping_shim(_input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError> {
    let val: u32 = 0xDEAD_BEEF;
    let s = postcard::to_slice(&val, output).map_err(|_| DispatchError::SerializeError)?;
    Ok(s.len())
}

static COMMANDS: [CommandMetadata; 1] = [CommandMetadata {
    name: "ping",
    id: 0x0001,
    invoke: ping_shim,
}];

let mut server = TelepathServer::<MyTransport, 512>::new(transport, &COMMANDS);
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

- `handle_discovery` is a TODO stub. Calling `cmd_id = 0x0000` succeeds
  with an empty payload but does not enumerate registered commands yet
  (roadmap [B4](https://github.com/tarotene/telepath/issues/3)).
- Command registry is a manually-passed `&'static [CommandMetadata]`.
  Distributed-slice auto-collection via `linkme` is planned (roadmap B3).
- The `#[command]` attribute is a passthrough stub; shims and metadata
  must be hand-written for now.
