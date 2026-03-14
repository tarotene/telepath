# telepath-cli

Host-side CLI for the Telepath RPC system. Connects to a target running
`telepath-firmware` (e.g. nRF52840-DK) via a J-Link or CMSIS-DAP debug
probe using probe-rs, attaches to RTT channel 1, and issues Telepath RPC
calls.

## Prerequisites

- A debug probe connected to the target (J-Link, CMSIS-DAP, ST-Link, etc.)
- [probe-rs](https://probe.rs/) installed and the probe driver set up
- Target firmware flashed and running (see `examples/nrf52840-dk`)

## Build

```
cargo build -p telepath-cli
```

## Usage

```
telepath-cli [--chip CHIP] <COMMAND>
```

| Option | Default | Description |
|--------|---------|-------------|
| `--chip` | `nRF52840_xxAA` | probe-rs chip name |

### Subcommands

#### `ping`

Send a ping request (CmdID `0x0001`) and print the returned `u32`.

```
telepath-cli ping
```

Expected output:

```
ping -> 0xDEADBEEF
```

With a non-default chip:

```
telepath-cli --chip STM32F411RETx ping
```

## RTT channel layout

The CLI uses RTT channel 1 for Telepath RPC traffic. Channel 0 is used
by the firmware for debug printing and is not accessed by the CLI.

| Channel | Direction | Purpose |
|---------|-----------|---------|
| 0 (up) | Target→Host | Debug prints (`rprintln!`) |
| 1 (up) | Target→Host | Telepath responses |
| 1 (down) | Host→Target | Telepath requests |
