# telepath — unified CLI

Host-side CLI for Telepath RPC servers. Provides two subcommands:

- **`telepath shell`** — interactive REPL and one-shot command runner
- **`telepath mcp`** — MCP server that exposes every `#[command]` function as an MCP tool

Both subcommands share the same transport layer and discover commands at runtime via the Command Discovery Protocol — no hardcoded command definitions required.

For a hardware-free smoke run, see [Quickstart in the root README](../../README.md#quickstart).

## Build

This crate is **excluded from the workspace**. Always `cd` into `tools/telepath` first.

```bash
# Default: shell + mcp subcommands, RTT transport
cd tools/telepath && cargo build

# Serial transport only (e.g. for USB-CDC or PTY)
cd tools/telepath && cargo build --no-default-features --features shell,serial

# MCP subcommand only, serial transport
cd tools/telepath && cargo build --no-default-features --features mcp,serial
```

## Subcommand: `shell`

Interactive REPL that discovers commands from the connected server and builds a prompt automatically.

```bash
# Interactive REPL (RTT, default chip nRF52840_xxAA)
cd tools/telepath && cargo run -- shell

# One-shot: run a command and exit
cd tools/telepath && cargo run -- shell --exec <command> [args...]

# Serial transport (e.g. PTY slave or USB-CDC)
cd tools/telepath && cargo run --no-default-features --features shell,serial -- \
    shell --transport serial --port /dev/ttyACM0

# Different chip
cd tools/telepath && cargo run -- shell --chip STM32F4
```

### Transport flags

| Flag | Default | Description |
|------|---------|-------------|
| `--transport rtt\|serial` | `rtt` | Transport backend |
| `--chip <NAME>` | `nRF52840_xxAA` | Chip name (RTT only) |
| `--rtt-control-block-addr <HEX>` | `0x20000000` | RTT control block address (RTT only); overridable via `TELEPATH_RTT_CONTROL_BLOCK_ADDR` |
| `--no-reset` | — | Disable automatic chip reset when RTT control block is not found on attach (RTT only) |
| `--port <PATH>` | — | Serial port path (serial only) |
| `--baud <N>` | `115200` | Baud rate (serial only) |

### REPL usage

After the server is connected, commands are auto-discovered and become available as REPL commands. Positional argument syntax is canonical:

```
telepath> <command>
telepath> <command> <arg1> <arg2>
```

JSON-array syntax (`<command> [arg1, arg2]`) is also accepted for backwards compatibility.

Type `help` for a full command list. Type `help <command>` for per-command usage.

## Subcommand: `mcp`

Starts an MCP server (stdio transport) that:

1. Connects to the Telepath RPC server using the selected transport
2. Runs the Command Discovery Protocol to enumerate all `#[command]` functions
3. Registers each command as an MCP tool with a JSON Schema derived from its postcard schema
4. Accepts MCP tool-call requests and bridges them to the RPC server

```bash
# MCP server (RTT, default chip)
cd tools/telepath && cargo run -- mcp

# MCP server over serial
cd tools/telepath && cargo run --no-default-features --features mcp,serial -- \
    mcp --transport serial --port /dev/ttyACM0
```

All available commands are discovered at runtime — no firmware-specific configuration is needed in this tool. See [`docs/mcp-integration.md`](../../docs/mcp-integration.md) for integration details including Claude Code setup.

## Logging

`telepath shell` writes RTT channel 0 (firmware debug logs) to:

- `$XDG_STATE_HOME/telepath/shell.log` if `$XDG_STATE_HOME` is set
- `~/.local/state/telepath/shell.log` otherwise

Redirect with `--log-file <PATH>` or suppress with `--log-file /dev/null`.

`telepath mcp` sends all diagnostic logs to **stderr**; stdout is reserved for the MCP JSON-RPC stream.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `shell` | ✓ | `telepath shell` subcommand (requires `rustyline`) |
| `mcp` | ✓ | `telepath mcp` subcommand (requires `tokio`, `rmcp`) |
| `rtt` | ✓ | probe-rs RTT transport |
| `serial` | — | CDC-ACM / PTY serial transport |
