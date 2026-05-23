# nrf52840-ping

Reference **server** deployment on nRF52840-DK (Embassy + RTT transport).

Minimal Embassy firmware for the [nRF52840-DK](https://www.nordicsemi.com/Products/Development-hardware/nRF52840-DK)
that demonstrates the Telepath RPC stack over RTT.  Registers four commands
that expose all on-board LEDs and buttons, and calls `server.poll()` in a
tight loop to handle incoming RPC requests.

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
> `examples/nrf52840-ping/` directory so that `.cargo/config.toml` (which
> sets the target and runner) is picked up correctly.

## Build

```
cd examples/nrf52840-ping
cargo build --release
```

## Flash

```
cd examples/nrf52840-ping
cargo run --release
```

`cargo run` invokes `probe-rs download` via the runner configured in
`.cargo/config.toml`. The firmware is written to flash, the chip resets,
and the probe session is released immediately. The terminal returns to the
shell prompt — the probe is free for `telepath-shell` to attach.

## Commands

Commands are registered via the `#[command]` macro and auto-discovered at
runtime by any connected host (MCP server, shell, or client library).

| Command | Signature | Description |
|---------|-----------|-------------|
| `ping` | `() -> u32` | Sanity check; returns `0xDEADBEEF`. |
| `led_set` | `(id: u8, on: bool) -> bool` | Set one LED. `id` 1–4; out-of-range returns `false`. |
| `led_pattern` | `(mask: u8) -> u8` | Set all four LEDs: bit 0 = LED1, bit 3 = LED4. Returns applied mask. |
| `led_pattern_get` | `() -> u8` | Read back driven state of all four LEDs. Bit 0 = LED1, bit 3 = LED4. On = 1, Off = 0. |
| `button_read` | `() -> u8` | Instantaneous button snapshot: bit 0 = BTN1, bit 3 = BTN4, pressed = 1. |

All four LEDs (LED1–LED4) are fully under RPC control.

### Button polling pattern

`button_read` returns a point-in-time snapshot — there is no `button_wait`
variant.  Blocking inside the synchronous `#[command]` dispatch loop is
forbidden by the Embassy executor model.  Callers that need edge detection
should poll `button_read` in a loop at the desired rate.

## RTT channel layout

| Channel | Direction | Purpose |
|---------|-----------|---------|
| 0 (up) | Target→Host | Debug prints via `rprintln!` and `hb {n}` heartbeat |
| 1 (up) | Target→Host | Telepath RPC responses |
| 1 (down) | Host→Target | Telepath RPC requests |

Channel 0 is connected to `rtt-target`'s print channel.  Approximately once
per second the firmware emits `hb {n}` (incrementing counter) as a liveness
indicator.  Channel 1 carries COBS-framed postcard-serialized Telepath frames.

## Verify with telepath-shell

Flash the firmware first (probe is released immediately after `cargo run --release` exits):

```
cd examples/nrf52840-ping && cargo run --release
```

Then launch the discovery-driven REPL.  `telepath-shell` calls the Command
Discovery Protocol (CmdID `0x0000`) at startup and builds a schema-aware
prompt from the registered commands — no hardcoded subcommands required.

```
telepath-shell
```

**Regression — `ping`:**

```
telepath> ping
ping -> 0xDEADBEEF
```

**LED set/get closed loop:**

```
telepath> led_pattern_get
led_pattern_get -> 0
telepath> led_pattern 10
led_pattern -> 10
telepath> led_pattern_get
led_pattern_get -> 10
telepath> led_set 1 true
led_set -> true
telepath> led_pattern_get
led_pattern_get -> 11
telepath> led_pattern 0
led_pattern -> 0
telepath> led_pattern_get
led_pattern_get -> 0
```

**Button snapshot:**

```
telepath> button_read
button_read -> 0
```

(Hold a button, then call `button_read` again — the corresponding bit will be set.)
