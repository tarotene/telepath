# telepath-host

Host-side Telepath RPC client (`std`). Connects to a target running
`telepath-firmware` via any byte-stream transport (`std::io::Read + Write`).

## Key API

### `TelepathClient<T>`

| Method | Description |
|--------|-------------|
| `new(transport)` | Wrap a transport (e.g. serialport, RTT adapter) |
| `call_raw(cmd_id, args)` | Send a request, block until response; returns payload bytes |
| `discover()` | Run the Command Discovery Protocol (CDP); populates schema cache |
| `schema_cache()` | Borrow the in-memory schema cache |

`T` must implement `std::io::Read + std::io::Write`.

### `HostError` variants

| Variant | Cause |
|---------|-------|
| `Io(String)` | Transport read/write failure |
| `SeqMismatch { expected, got }` | Response sequence number mismatch |
| `SystemError` | Target reported a system-level error |
| `AppError(Vec<u8>)` | Target reported an application-level error |
| `SerdeError` | postcard serialization/deserialization failure |
| `RequestPayloadTooLarge` | `args` exceeded `MAX_PAYLOAD_SIZE` (256 bytes) |
| `ResponsePayloadTooLarge` | Response payload exceeded `MAX_PAYLOAD_SIZE` |
| `FrameTooLarge` | Received frame exceeded `MAX_FRAME_SIZE` (512 bytes) |
| `FramingError` | Malformed COBS frame from target |

### `SchemaCache`

In-memory map from `cmd_id` to `SchemaEntry` (name, arg schema, return
schema). Cache coherence is guaranteed by the `cmd_id` design: the ID is
a hash of (name + input schema + output schema), so any type change
produces a new ID automatically.

## Usage

```rust
use telepath_host::TelepathClient;

// transport: anything implementing std::io::Read + std::io::Write
let mut client = TelepathClient::new(transport);

// Ping (CmdID 0x0001, no args)
let payload = client.call_raw(0x0001, &[])?;
let val: u32 = postcard::from_bytes(&payload)?;
println!("ping -> 0x{:08X}", val);  // ping -> 0xDEADBEEF
```

## Build

```
cargo build -p telepath-host
cargo test -p telepath-host
```

## Limitations

- `discover()` returns `Ok(0)` and does not populate `SchemaCache`. The
  CDP wire format is being finalised (roadmap [B5](https://github.com/tarotene/telepath/issues/3)).
- No typed `call::<Args, Ret>` API yet — only `call_raw(cmd_id, &[u8])`
  (roadmap C1).
- Upstream rzCOBS is not yet supported; both framing directions are COBS.
- `HostError::SerdeError` is opaque — the original `postcard::Error` is
  discarded (roadmap C3).
