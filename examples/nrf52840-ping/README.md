# nrf52840-ping

Reference **server** deployment on nRF52840-DK (Embassy + RTT transport).

Minimal Embassy firmware for the [nRF52840-DK](https://www.nordicsemi.com/Products/Development-hardware/nRF52840-DK)
that demonstrates the Telepath RPC stack over RTT.  Registers nine commands
covering on-board LEDs, buttons, and on-chip sensor/ID peripherals unique to
real silicon, then calls `server.poll()` in a tight loop to handle incoming
RPC requests.

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
shell prompt — the probe is free for `telepath shell` to attach.

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
| `ficr_uid` | `() -> (u32, u32)` | Factory-programmed unique 64-bit chip ID from FICR.DEVICEID[0..1]. Stable across reboots; different on every board. |
| `temp_read` | `() -> i16` | Die temperature in 0.25 °C units from the on-chip TEMP peripheral. Divide by 4 for °C (e.g. 100 → 25 °C). Operating range: −160…340 (−40 °C to 85 °C). |
| `rng_u32` | `() -> u32` | Hardware true random u32 with bias correction. Each call produces 4 fresh bytes from the silicon RNG; values should differ across calls. |
| `saadc_vdd_mv` | `() -> u16` | Supply voltage in millivolts via the SAADC VDD internal channel. ~3300 mV under USB power; ~3000 mV from a coin cell. |

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

## Verify with telepath shell

Flash the firmware first (probe is released immediately after `cargo run --release` exits):

```bash
cd examples/nrf52840-ping && cargo run --release
```

Then launch the discovery-driven REPL.  `telepath shell` calls the Command
Discovery Protocol (CmdID `0x0000`) at startup and builds a schema-aware
prompt from the registered commands — no hardcoded subcommands required.

```bash
cd tools/telepath && cargo run -- shell
```

**Regression — `ping`:**

```
telepath> ping
ping -> 0xDEADBEEF
```

**CPU-only commands — `add` / `crc32` / `echo`:**

Positional syntax is canonical (space-separated args). JSON-array form
(`[arg1, arg2, ...]`) is also accepted, where the outer array is the argument
list (one element per parameter). For commands whose single argument is a byte
array (`crc32`, `echo`) the byte array itself must be wrapped in the
argument-list outer array: `[[byte1, byte2, ...]]`. The encoder treats any
JSON array passed to a 1-parameter command as the argument-list container, not
as the argument value, so a bare `[byte1, byte2, ...]` would be interpreted as
a list of individual arguments and fail with an arity error.

```
telepath> add 2 3
add -> 5
telepath> add -1 1
add -> 0
telepath> crc32 [[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]]
crc32 -> 0xC2A8FA9D
telepath> echo [[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127]]
echo -> [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127]
```

The `crc32` reference value `0xC2A8FA9D` is CRC-32/ISO-HDLC over 128 zero bytes —
verified independently with `python3 -c "import zlib; print(hex(zlib.crc32(bytes(128)) & 0xFFFFFFFF))"`.

**LED set/get closed loop:**

```
telepath> led_pattern_get
led_pattern_get -> 0x00
telepath> led_pattern 10
led_pattern -> 0x0A
telepath> led_pattern_get
led_pattern_get -> 0x0A
telepath> led_set 1 true
led_set -> true
telepath> led_pattern_get
led_pattern_get -> 0x0B
telepath> led_pattern 0
led_pattern -> 0x00
telepath> led_pattern_get
led_pattern_get -> 0x00
```

**Button snapshot:**

```
telepath> button_read
button_read -> 0x00
```

(Hold a button, then call `button_read` again — the corresponding bit will be set.)

**Chip ID — `ficr_uid`:**

The two `u32` values form a 64-bit unique ID.  They are factory-programmed and
stable across reboots.  A different board will show different values.

> **HIL reference** — one particular nRF52840-DK returned `[0x893CE2C0, 0xA94DF961]`
> (decimal: `[2302468800, 2840459617]`).  Your board's values will differ.

**Die temperature — `temp_read`:**

```
telepath> temp_read
temp_read -> 100
```

Divide the raw value by 4 to get °C: 100 → 25.00 °C, 111 → 27.75 °C.  Run
`temp_read` before and after a tight loop to observe self-heating — the value
should increase by several counts.  Acceptance range: −160…340 (nRF52840
operating temperature −40 °C to 85 °C).

> **HIL reference** — idle room-temperature reading on one board: `111` (27.75 °C).

**Hardware RNG — `rng_u32`:**

```
telepath> rng_u32
rng_u32 -> 0x7F3A19C2
telepath> rng_u32
rng_u32 -> 0xB40E5D87
```

Values should differ on every call.  Bias correction is enabled (set via
`RNG.CONFIG.DERCEN = 1`) so the output is free from bias toward 0 or 1.

**Supply voltage — `saadc_vdd_mv`:**

```
telepath> saadc_vdd_mv
saadc_vdd_mv -> 3265
```

Under USB power expect 3000–3300 mV.  Switch to a coin cell and the value
drops noticeably, demonstrating the command's ability to detect power-source
changes at runtime.

> **HIL reference** — 5 consecutive samples under USB power: `2981, 2984, 2984,
> 2988, 2991` mV (avg ≈ 2986 mV ≈ 3.0 V).  Formula: `raw_count × 3600 / 1024`
> (10-bit ADC, GAIN=1/6, VREF=0.6 V → full-scale = 3.6 V).
