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

## Notes

- This crate is **excluded from the workspace** — always `cd` into it before
  running `cargo` commands.
- `stdout` carries the MCP JSON-RPC stream; all logging goes to `stderr`.
- Only the loopback transport is implemented; RTT/serialport is future work.
