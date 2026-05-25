# telepath-shell

**Interactive shell** for a Telepath server: REPL and one-shot commands.

Host-side interactive shell for the Telepath RPC system. Transport is selected
**at build time** via Cargo features:

| Feature | Transport | Build command |
|---------|-----------|---------------|
| `rtt` *(default)* | probe-rs RTT to a flashed device | `cargo build` |
| `serial` | CDC-ACM serial port or PTY | `cargo build --no-default-features --features serial` |

In RTT mode, channel 0 debug output (firmware `rprintln!` calls) is forwarded to
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

**RTT build** (default):

```
telepath-shell [--chip CHIP] [--rtt-control-block-addr ADDR] [--no-reset] [--log-file PATH] [--exec COMMAND...]
```

**Serial build** (`--no-default-features --features serial`):

```
telepath-shell --port PORT [--baud RATE] [--exec COMMAND...]
```

| Option | Transport | Default | Description |
|--------|-----------|---------|-------------|
| `--chip` | RTT | `nRF52840_xxAA` | probe-rs chip name |
| `--rtt-control-block-addr` | RTT | `0x20000000` | RTT control block address (hex); also via `TELEPATH_RTT_CONTROL_BLOCK_ADDR` |
| `--no-reset` | RTT | disabled | Skip automatic chip reset retry when RTT control block is missing on attach |
| `--log-file` | RTT | `~/.local/state/telepath/shell.log` | Destination for RTT ch0 debug logs; `-` for stderr, `/dev/null` to suppress |
| `--port` | serial | *(required)* | Serial port path (e.g. `/dev/ttyACM0`, `/dev/pts/N`) |
| `--baud` | serial | `115200` | Serial baud rate |
| `--exec` | both | — | Execute a single command non-interactively and exit (same syntax as REPL) |

In RTT mode, if the firmware has not yet initialized the SEGGER RTT control block when `telepath-shell`
attaches, the shell issues a soft chip reset and retries once. Pass `--no-reset` to disable this.

### 1-shot mode

Use `--exec` to run a single command non-interactively and exit:

```
cd tools/telepath-shell && cargo run -- --exec ping
```

With a non-default chip:

```
cd tools/telepath-shell && cargo run -- --chip nRF52840_xxAA --exec ping
```

Multi-argument commands use the same syntax as the REPL prompt:

```
cd tools/telepath-shell && cargo run -- --exec add 2 3
```

Any probe-rs chip identifier is accepted; run `probe-rs chip list` to find yours.

### Interactive REPL mode

Omit the subcommand to enter a readline prompt:

```
cd tools/telepath-shell && cargo run
```

At startup the shell calls the Command Discovery Protocol and builds a
schema-aware prompt from the registered commands — no hardcoded subcommands.

```
telepath> help
Commands:
  ping               -> u32
  add <i32> <i32>    -> i32
  ...
  help [COMMAND]     Show this help or detail for a command

telepath> ping
ping -> 0xDEADBEEF

telepath> add [2, 3]
add -> 5
```

RTT debug output from the firmware appears on stderr before each prompt.
Exit with `quit`, `exit`, Ctrl-C, or Ctrl-D.

### Serial REPL (hardware-free via `host-pty-server`)

Build the serial variant and point it at the slave PTY exposed by `host-pty-server`:

```
cd tools/telepath-shell && cargo build --no-default-features --features serial
cargo run -p host-pty-server   # prints HOST_PTY_SERVER_PATH=/dev/pts/N
cd tools/telepath-shell && cargo run --no-default-features --features serial -- --port /dev/pts/N
```

## RTT channel layout

| Channel | Direction | Purpose |
|---------|-----------|---------|
| 0 (up) | Target→Host | Debug prints (`rprintln!`) — forwarded to stderr |
| 1 (up) | Target→Host | Telepath responses |
| 1 (down) | Host→Target | Telepath requests |

