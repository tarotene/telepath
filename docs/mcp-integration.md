# MCP Integration — `tools/telepath-mcp-server`

`telepath-mcp-server` exposes every `#[command]` function on a connected
Telepath server as an MCP tool, with zero hand-written tool descriptors.

## Architecture

```
[MCP Client (AI agent)]
        ↕ stdio JSON-RPC
[telepath-mcp-server bin]
   ├ server.rs   — rmcp::ServerHandler, list_tools / call_tool
   ├ bridge.rs   — async invoke(): JSON args → postcard → call_raw → JSON ret
   ├ schema_to_json.rs   — OwnedNamedType → JSON Schema (pure, sync)
   ├ json_to_postcard.rs — serde_json::Value + schema → postcard bytes (pure, sync)
   └ postcard_to_json.rs — postcard bytes + schema → serde_json::Value (pure, sync)
        ↕ TelepathClient::call_raw
[Telepath server (loopback or RTT)]
```

The three pure modules (`schema_to_json`, `json_to_postcard`, `postcard_to_json`)
have no I/O or async dependencies and are covered by unit tests.

## Startup sequence

```
1. client.discover()         — fetches all #[command] metadata via CDP paging
2. For each SchemaEntry:
   a. decoded_args_schema()  — postcard::from_bytes → OwnedNamedType
   b. named_type_to_json_schema() → serde_json::Value (JSON Schema)
   c. rmcp::Tool::new(name, description, input_schema)
3. serve((stdin, stdout))    — rmcp stdio JSON-RPC loop
```

## OwnedDataModelType → JSON Schema mapping

| `OwnedDataModelType` | JSON Schema |
|---|---|
| `Bool` | `{"type":"boolean"}` |
| `U8` | `{"type":"integer","minimum":0,"maximum":255}` |
| `U16` | `{"type":"integer","minimum":0,"maximum":65535}` |
| `U32` | `{"type":"integer","minimum":0,"maximum":4294967295}` |
| `U64` | `{"type":"integer","minimum":0,"maximum":18446744073709551615}` |
| `U128` | `{"type":"string","pattern":"^[0-9]+$"}` (JSON cannot represent u128 natively) |
| `Usize` | same as U64 |
| `I8` | `{"type":"integer","minimum":-128,"maximum":127}` |
| `I16/I32/I64` | bounded integer accordingly |
| `I128` | `{"type":"string","pattern":"^-?[0-9]+$"}` |
| `Isize` | same as I64 |
| `F32` / `F64` | `{"type":"number"}` |
| `Char` | `{"type":"string","minLength":1,"maxLength":1}` |
| `String` | `{"type":"string"}` |
| `ByteArray` | `{"type":"array","items":{"type":"integer","minimum":0,"maximum":255}}` |
| `Option(T)` | `{"oneOf":[<T>,{"type":"null"}]}` |
| `Unit` / `UnitStruct` | `{"type":"null"}` |
| `NewtypeStruct(T)` | transparent: `<T>` |
| `Seq(T)` | `{"type":"array","items":<T>}` |
| `Tuple([T1,…,Tn])` / `TupleStruct` | `{"type":"array","prefixItems":[…],"minItems":n,"maxItems":n}` |
| `Map{key=String,val:V}` | `{"type":"object","additionalProperties":<V>}` |
| `Map{key≠String,val:V}` | `{"type":"array","items":{"prefixItems":[<K>,<V>],"minItems":2,"maxItems":2}}` |
| `Struct(fields)` | `{"type":"object","properties":{…},"required":[…],"additionalProperties":false}` |
| `Enum` (all unit variants) | `{"type":"string","enum":[…]}` |
| `Enum` (mixed variants) | `{"oneOf":[…]}` (Serde external-tag form: `{"VariantName":payload}`) |
| `Schema` | `{"type":"object","description":"opaque postcard schema"}` |

Recursion depth is capped at 8. Exceeding it produces
`{"type":"object","description":"schema depth exceeded"}`.

## JSON ↔ postcard encoding contract

Encoding follows the postcard wire format exactly:

- Integer types use postcard's varint compression for `u16`/`u32`/`u64`/`i16`/`i32`/`i64`.
  **Never compare encoded bytes to hand-crafted literals** — always use
  `postcard::to_allocvec(&native_value)` as the oracle.
- `u128`/`i128` are passed as decimal strings in JSON (e.g. `"340282366920938463463374607431768211455"`).
- `ByteArray` encodes as a postcard length-prefixed byte slice; JSON side uses an
  integer array `[0, 255, …]`.
- `Option`: `null` → `[0x00]`; any other value → `[0x01, <encoded inner>]`.
- `Unit` / `UnitStruct` emits no bytes; JSON accepts any value.
- `Enum`: discriminant is `u32` (varint); payload follows the variant layout.
- Multi-arg `#[command]` functions encode args as a postcard tuple `(T1, T2, …)`,
  which maps to a JSON array `[v1, v2, …]` — not an object.

## Edge cases

| Scenario | Behaviour |
|---|---|
| Zero-arg `#[command]` | args schema = `Unit`; accepts any JSON, emits 0 bytes |
| Single-arg `#[command]` | args schema = `(T,)`; JSON = `[v]` (one-element array) |
| Multi-arg `#[command]` | args schema = `(T1,T2,…)`; JSON = `[v1,v2,…]` |
| Struct return type | ret schema = `Struct(fields)`; JSON = `{"field":value,…}` |
| Enum return type | unit variants return `"Name"` string; payload variants return `{"Name":payload}` |

## Running

```bash
# Loopback mode — no hardware required (demo ping command built in)
cd tools/telepath-mcp-server
cargo run -- --transport loopback
```

## Known limitations and followups

- RTT/serialport transport (`--transport rtt`, `--transport serial:/dev/ttyUSB0`) — see #36
- Schema cache invalidation on firmware reconnect — see #37
- Named-argument mapping for tuple-schema commands (bridge-level adaptation) — see #38
- MCP `resources` / `prompts` capability exposure — see #39
- Shared `telepath-testing` crate to consolidate loopback infrastructure — see #40
