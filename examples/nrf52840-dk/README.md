# nrf52840-dk example

Minimal Embassy firmware for the [nRF52840-DK](https://www.nordicsemi.com/Products/Development-hardware/nRF52840-DK)
that demonstrates the Telepath RPC stack over RTT.

Registers a single `ping` command (CmdID `0x0001`) that returns
`0xDEADBEEF: u32`, blinks LED 1 to indicate liveness, and calls
`server.poll()` in a tight loop to handle incoming RPC requests.

## Prerequisites

- Rust stable toolchain with the `thumbv7em-none-eabi` target:
  ```
  rustup target add thumbv7em-none-eabi
  ```
- [probe-rs](https://probe.rs/) for flashing and RTT:
  ```
  cargo install probe-rs-tools
  ```
- nRF52840-DK connected via USB (J-Link on-board)

> **Note:** This crate is excluded from the workspace (`exclude` in the
> root `Cargo.toml`). Always run build commands from within the
> `examples/nrf52840-dk/` directory so that `.cargo/config.toml` (which
> sets the target and runner) is picked up correctly.

## Build

```
cd examples/nrf52840-dk
cargo build --release
```

## Flash and run

```
cd examples/nrf52840-dk
cargo run --release
```

`cargo run` invokes probe-rs via the runner configured in
`.cargo/config.toml`. The firmware starts immediately after flashing.
RTT output appears in the terminal.

## RTT channel layout

| Channel | Direction | Purpose |
|---------|-----------|---------|
| 0 (up) | Target→Host | Debug prints via `rprintln!` |
| 1 (up) | Target→Host | Telepath RPC responses |
| 1 (down) | Host→Target | Telepath RPC requests |

Channel 0 is connected to `rtt-target`'s print channel, so `rprintln!`
output is visible in the RTT console. Channel 1 carries COBS-framed
postcard-serialized Telepath frames and is consumed by `telepath-cli`.

## Verify with telepath-cli

With the firmware running, open a second terminal:

```
cargo build -p telepath-cli
./target/debug/telepath-cli ping
```

Expected output:

```
ping -> 0xDEADBEEF
```
