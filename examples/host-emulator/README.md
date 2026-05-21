# host-emulator

In-process Telepath round-trip — no hardware required.

Spawns a `TelepathServer` on one OS thread and drives it from a
`TelepathClient` on the main thread, with two `std::sync::mpsc` byte
channels standing in for the physical wire.

## Run

```
cargo run -p host-emulator
```

## Expected output

```
ping -> 0xDEADBEEF
```

## What this demonstrates

| Layer | Real hardware (nRF52840-DK) | This emulator |
|---|---|---|
| Transport | probe-rs RTT channel 1 | `std::sync::mpsc::sync_channel<u8>` pair |
| Framing | COBS, delimiter `0x00` | Identical — raw COBS bytes traverse the channel |
| Serialization | postcard | Identical |
| Server | `TelepathServer` over `RttTransport` | `TelepathServer` over `FwSideTransport` |
| Client | `TelepathClient` over RTT | `TelepathClient` over `HostSideTransport` |

The same `telepath-firmware` and `telepath-host` code paths execute as on
real hardware. Switching to a real MCU is purely a transport swap — the
framing and serialization layers are unchanged.

## Code structure

```
src/
├── main.rs       — ping shim, COMMANDS slice, two-thread orchestration
└── loopback.rs   — FwSideTransport (Transport impl) / HostSideTransport (Read+Write impl)
```

The design asymmetry is intentional: `FwSideTransport::read` uses
`try_recv()` (non-blocking, returns `0` when empty) to match the
`Transport` contract; `HostSideTransport::read` blocks on `recv()` for
the first byte, which is what `TelepathClient::call_raw` expects from
a `std::io::Read` transport.

## Limitations

- One round-trip per run. Extend `main.rs` to issue more `call_raw` calls;
  the loopback channels survive multiple sequential requests.
- Realtime latency, RTT-control-block contention, and partial-write
  behaviour on embedded hardware are not modelled. Use
  `examples/nrf52840-dk` for those scenarios.
- This example only exercises `ping`; it does not call `discover()`.
  The full discovery path is exercised in
  `crates/telepath-firmware/tests/discovery_smoke.rs`.
