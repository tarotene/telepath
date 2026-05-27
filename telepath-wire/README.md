# telepath-wire

Shared wire protocol types for the Telepath RPC system. Used by both
`telepath-server` (MCU side) and `telepath-client` (PC side).

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
| `WireError` | `PayloadTooLarge` / `SerdeError(postcard::Error)` / `UnknownPacketType` / `FramingError` |

## Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `MAX_PAYLOAD_SIZE` | 256 | Maximum payload bytes (both sides enforce this) |
| `framing::MAX_FRAME_SIZE` | 512 | Maximum COBS frame bytes including delimiter |
| `CMD_ID_DISCOVERY` | `0x0000` | Reserved CmdID for Command Discovery Protocol |

## Command ID derivation

Command IDs are 16-bit values that identify a command on the wire. Each ID is
derived deterministically from the command's textual signature at build time,
so firmware and host always agree without a runtime registry sync.

### Algorithm

FNV-1a 32-bit (offset basis `0x811c9dc5`, prime `0x01000193`), XOR-folded to
16 bits: `result = (hash >> 16) ^ (hash as u16)`.

XOR-fold is preferred over truncation because it preserves avalanche across the
full input range and reduces low-bit bias inherent in multiplicative hashes.

### Pre-image

```text
name || 0x1F || args_type || 0x1F || ret_type   (UTF-8 bytes)
```

`0x1F` (ASCII Unit Separator) cannot appear in Rust identifiers or type paths,
so it is collision-free as a field delimiter.

### Type-name caveat

`args_type` and `ret_type` are the textual Rust type names as extracted by the
`#[command]` proc-macro (`syn`-derived token strings). This is a **textual**
canonicalization, not a true postcard schema digest:

- Renaming `struct Foo { x: u8 }` to `struct Bar { x: u8 }` **changes** the ID.
- Reordering fields inside `Foo` does **not** change the ID.

Migration to a real `postcard-schema` fingerprint is planned once
`postcard >= 1.2` is adopted in this workspace
(see [issue #3](https://github.com/tarotene/telepath/issues/3)).

### 0x0000 reservation

`CMD_ID_DISCOVERY` (`0x0000`) is reserved for the Command Discovery Protocol.
If the raw hash collides with it, `derive_cmd_id` loops over descending salt
bytes (`0xFF`, `0xFE`, …) until the result is non-zero — guaranteeing that
`CMD_ID_DISCOVERY` is never returned.

### Collision risk

| Commands (N) | P(at least one collision) |
|-------------|--------------------------|
| 32          | ~0.8%                    |
| 64          | ~3.1%                    |
| 128         | ~12%                     |
| 256         | ~38%                     |
| 1024        | ~99%                     |

(Birthday-paradox approximation: P ≈ 1 − e^(−N²/131072).)

Keep one device's command count **≤ 64** for a comfortable collision margin.
The reserved `0x0000` ID is avoided by rehashing the preimage with a `0xFF` salt byte when the raw hash equals `0x0000`; the discovery ID is never emitted by user commands.
Cross-command duplicate ID detection is enforced at build time: two `#[command]` functions in the same crate that hash to the same ID produce a `compile_error!`, while cross-crate collisions are caught as a linker "multiple definition" error before the firmware is flashed.

### Usage

```rust
use telepath_wire::cmd_id::{derive_cmd_id, fnv1a_16};

// Derive a cmd_id from a function signature.
const CMD_PING: u16 = derive_cmd_id("ping", "()", "u32");

// Raw FNV-1a 16-bit hash.
const H: u16 = fnv1a_16(b"hello");
```

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

## Limitations

- Upstream (target → host) framing uses COBS in this MVP. rzCOBS is
  planned for Stage C2 (see [#76](https://github.com/tarotene/telepath/issues/76)).
- `AppError` payload format is unspecified. Until resolved, callers must
  agree on an out-of-band convention
  (see [#78](https://github.com/tarotene/telepath/issues/78)).
