# telepath-cli

Host-side CLI for the Telepath RPC system. Connects to a target running
`telepath-firmware` (e.g. nRF52840-DK) via a J-Link or CMSIS-DAP debug
probe using probe-rs, attaches to RTT, and issues Telepath RPC calls.

RTT channel 0 debug output (firmware `rprintln!` calls) is forwarded to
stderr automatically. RTT channel 1 carries Telepath RPC traffic.

## Prerequisites

- A debug probe connected to the target (J-Link, CMSIS-DAP, ST-Link, etc.)
- [probe-rs](https://probe.rs/) installed and the probe driver set up
- Target firmware flashed and running (see `examples/nrf52840-dk`)
- The probe must be free — `cargo run --release` in the firmware example
  now uses `probe-rs download` (flash + exit), so the probe is released
  before this tool is invoked

## Build

```
cd tools/telepath-cli && cargo build
```

## Usage

```
telepath-cli [--chip CHIP] [COMMAND]
```

| Option | Default | Description |
|--------|---------|-------------|
| `--chip` | `nRF52840_xxAA` | probe-rs chip name |

### 1-shot mode

Pass a subcommand to execute it and exit:

```
cd tools/telepath-cli && cargo run -- ping
```

With a non-default chip:

```
cd tools/telepath-cli && cargo run -- --chip STM32F411RETx ping
```

### Interactive REPL mode

Omit the subcommand to enter a readline prompt:

```
cd tools/telepath-cli && cargo run
```

```
telepath> ping
ping -> 0xDEADBEEF
telepath> help
Commands: ping, help, quit
telepath> quit
```

RTT debug output from the firmware appears on stderr before each prompt.
Exit with `quit`, `exit`, Ctrl-C, or Ctrl-D.

### Subcommands

#### `ping`

Send a ping request (CmdID `0x0001`) and print the returned `u32`.

Expected output:

```
ping -> 0xDEADBEEF
```

## RTT channel layout

| Channel | Direction | Purpose |
|---------|-----------|---------|
| 0 (up) | Target→Host | Debug prints (`rprintln!`) — forwarded to stderr |
| 1 (up) | Target→Host | Telepath responses |
| 1 (down) | Host→Target | Telepath requests |
