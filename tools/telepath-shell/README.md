# telepath-shell

**Interactive shell** for a Telepath server: RTT-attached REPL and one-shot commands.

Host-side interactive shell for the Telepath RPC system. Connects to a target running
`telepath-server` (e.g. nRF52840-DK) via a J-Link or CMSIS-DAP debug
probe using probe-rs, attaches to RTT, and issues Telepath RPC calls.

RTT channel 0 debug output (firmware `rprintln!` calls) is forwarded to
stderr automatically. RTT channel 1 carries Telepath RPC traffic.

## Prerequisites

- A debug probe connected to the target (J-Link, CMSIS-DAP, ST-Link, etc.)
- [probe-rs](https://probe.rs/) installed and the probe driver set up
- Target firmware flashed and running (see `examples/nrf52840-ping`)
- The probe must be free — `cargo run --release` in the firmware example
  now uses `probe-rs download` (flash + exit), so the probe is released
  before this tool is invoked

## Build

```
cd tools/telepath-shell && cargo build
```

## Usage

```
telepath-shell [--chip CHIP] [COMMAND]
```

| Option | Default | Description |
|--------|---------|-------------|
| `--chip` | `nRF52840_xxAA` | probe-rs chip name |
| `--no-reset` | (disabled) | Skip the automatic chip reset retry when the RTT control block is missing on attach |

If the firmware has not yet initialized the SEGGER RTT control block when `telepath-shell` attaches
(common right after `cargo run --release` in `examples/nrf52840-ping`), the shell issues a soft chip
reset and retries the attach once. Pass `--no-reset` to disable this behavior.

### 1-shot mode

Pass a subcommand to execute it and exit:

```
cd tools/telepath-shell && cargo run -- ping
```

With a non-default chip:

```
cd tools/telepath-shell && cargo run -- --chip nRF52840_xxAA ping
```

Any probe-rs chip identifier is accepted; run `probe-rs chip list` to find yours.

### Interactive REPL mode

Omit the subcommand to enter a readline prompt:

```
cd tools/telepath-shell && cargo run
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

## Limitations

- Only `ping` is implemented. `discover` and arbitrary command dispatch
  await the typed-API milestone (roadmap C1,
  [tracking issue #3](https://github.com/tarotene/telepath/issues/3)).
