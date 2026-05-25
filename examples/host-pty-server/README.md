# host-pty-server

Hardware-free regression server: full wire round-trip over a real PTY pair.

Opens a PTY with `openpty(3)`, prints the slave device path, then runs a
`TelepathServer` on the master side in a poll loop. A client (e.g.
`telepath shell --transport serial`) connects to the slave end and speaks the
full Telepath wire protocol — COBS framing + postcard serialization — over
the PTY byte stream.

## Run

```
cargo run -p host-pty-server
```

The process prints one line to stdout and then keeps running:

```
HOST_PTY_SERVER_PATH=/dev/pts/3
```

Connect a client to the printed path:

```bash
cd tools/telepath
cargo run --no-default-features --features shell,serial -- shell --transport serial --port /dev/pts/3 --exec ping
```

Or use `just host-pty-smoke` to run the full two-process smoke automatically.

## Expected output (smoke)

```
ping -> 0xDEADBEEF
```

## What this demonstrates

| Layer | Real hardware (nRF52840-DK) | This server |
|---|---|---|
| Transport | probe-rs RTT channel 1 | PTY master (`O_NONBLOCK` `File`) |
| Framing | COBS, delimiter `0x00` | Identical — raw COBS bytes traverse the PTY |
| Serialization | postcard | Identical |
| Server | `TelepathServer` over `RttTransport` | `TelepathServer` over `PtyTransport` |
| Client | `telepath shell --transport rtt` | `telepath shell --transport serial` |

The same `telepath-server` code path executes as on real hardware. Switching
to a real MCU is purely a transport swap — framing and serialization are
unchanged.

## Code structure

Single binary in `src/main.rs` containing the `#[command]` functions, the `PtyTransport` impl, and the `TelepathServer` poll loop.

## Registered commands

The same four CPU-only demo commands as `examples/nrf52840-ping`:

| Command | Args | Return |
|---------|------|--------|
| `ping` | — | `u32` (`0xDEAD_BEEF`) |
| `add` | `i32, i32` | `i32` |
| `crc32` | `heapless::Vec<u8, 128>` | `u32` |
| `echo` | `heapless::Vec<u8, 128>` | `heapless::Vec<u8, 128>` |
