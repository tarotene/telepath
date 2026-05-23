# telepath-mcp-server

MCP server that exposes every `#[command]` function on a connected Telepath
server as an MCP tool — zero hand-written tool descriptors required.

## Quick start (loopback, no hardware)

```bash
cd tools/telepath-mcp-server
cargo build
cargo run -- --transport loopback
```

The binary writes MCP JSON-RPC to `stdout` and reads from `stdin`.  Use MCP
Inspector for interactive testing:

```bash
npx @modelcontextprotocol/inspector ./target/debug/telepath-mcp-server --transport loopback
```

Expected: `ping` tool appears in the tool list; calling it returns
`3735928559` (`0xDEADBEEF`).

## Tests

```bash
cargo test
```

Three test suites:

| Suite | What it covers |
|---|---|
| `schema_to_json_table` | All `OwnedDataModelType` variants → JSON Schema mapping |
| `json_postcard_roundtrip` | encode → decode identity; native postcard oracle comparison |
| `end_to_end_loopback` | discover + invoke `ping` and `add` via full bridge stack |

## Architecture

See [`docs/mcp-integration.md`](../../docs/mcp-integration.md) for the full
architecture diagram and encoding contract.

## Using from Claude Code

`telepath-mcp-server` is an MCP server, so any MCP-compatible coding agent can use
it. The shortest path with [Claude Code](https://claude.com/claude-code):

### 1. Build the binary

```bash
cd tools/telepath-mcp-server
cargo build
```

### 2. Register as a project-scoped MCP server

```bash
claude mcp add --transport stdio --scope project telepath \
  -- "${CLAUDE_PROJECT_DIR}/tools/telepath-mcp-server/target/debug/telepath-mcp-server" \
  --transport loopback
```

This writes `.mcp.json` at the repository root (already committed to this repo).
The server loads automatically on every Claude Code session inside the repository —
no further setup needed after `cargo build`.

### 3. Verify

Start a new Claude Code session inside the repository and run `/mcp` to confirm
`telepath` appears with its discovered tools. For the loopback build, `ping` will
be listed.

### 4. Invoke a Telepath command

In a Claude Code prompt:

> Call the `ping` MCP tool and report the result.

Expected: the agent invokes the tool and returns `3735928559` (`0xDEADBEEF`).

## Notes

- This crate is **excluded from the workspace** — always `cd` into it before
  running `cargo` commands.
- `stdout` carries the MCP JSON-RPC stream; all logging goes to `stderr`.
- The loopback transport is currently the only built-in transport — see #36 for RTT/serialport support.
