# telepath-wire

Shared wire protocol types for the Telepath RPC system. Used by both
`telepath-firmware` (MCU side) and `telepath-host` (PC side).

`#![no_std]` — no heap allocation. All types are stack-friendly and
borrow from the receive buffer for zero-copy deserialization.

## Protocol overview

| Aspect | Specification |
|--------|---------------|
| Framing (Host→Target) | COBS; `0x00` delimiter |
| Framing (Target→Host) | COBS (rzCOBS planned) |
| Serialization | postcard (little-endian, varint-compressed) |
| Packet types | `Request` (0x01) / `Response` (0x02) |
| Error signaling | `ResponseStatus` field inside `Response` |
| Discovery CmdID | `0x0000` — reserved (CDP) |

## Key types

| Item | Description |
|------|-------------|
| `Request<'a>` | RPC call from host to target; `args` borrows from the receive buffer |
| `Response<'a>` | RPC reply from target to host; `payload` borrows from the receive buffer |
| `PacketType` | Wire discriminant: `Request` / `Response` |
| `ResponseStatus` | `Ok` / `AppError` / `SystemError` |
| `WireError` | `PayloadTooLarge` / `SerdeError` / `UnknownPacketType` / `FramingError` |

## Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `MAX_PAYLOAD_SIZE` | 256 | Maximum payload bytes (both sides enforce this) |
| `framing::MAX_FRAME_SIZE` | 512 | Maximum COBS frame bytes including delimiter |
| `CMD_ID_DISCOVERY` | `0x0000` | Reserved CmdID for Command Discovery Protocol |

## `framing` module

```rust
use telepath_wire::framing::{cobs_encode, cobs_decode, FrameAccumulator, MAX_FRAME_SIZE};

// Encode
let mut frame = [0u8; MAX_FRAME_SIZE];
let n = cobs_encode(data, &mut frame)?;  // includes 0x00 delimiter

// Decode
let mut decoded = [0u8; MAX_FRAME_SIZE];
let m = cobs_decode(&frame[..n - 1], &mut decoded)?;

// Stream accumulation
let mut acc = FrameAccumulator::<512>::new();
for byte in stream {
    if acc.feed(byte) {          // returns true on 0x00 delimiter
        let raw = acc.frame();   // Some(&[u8]) or None on overflow
        acc.reset();
    }
}
```

## Build

```
cargo build -p telepath-wire
cargo test -p telepath-wire
```
