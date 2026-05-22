# Telepath — Agent Configuration

> Project-specific rules for AI coding agents.
> RFC 2119 keywords (MUST, SHOULD, MAY) indicate requirement strength.

## Workspace Overview

| Crate | Role | Target |
|-------|------|--------|
| `crates/telepath-wire` | Shared wire protocol types | host + firmware |
| `crates/telepath-macros` | `#[command]` proc-macro | host (build time only) |
| `crates/telepath-firmware` | Target-side RPC server | `thumbv7em-none-eabi` |
| `crates/telepath-host` | Host-side RPC client | native (`std`) |
| `examples/host-emulator` | In-process server+client emulator | native (`std`) |
| `examples/nrf52840-dk` | Standalone firmware example | `thumbv7em-none-eabi` |
| `tools/telepath-cli` | Host-side CLI over RTT | native (`std`) |

## Build Commands

```
# Host workspace (all 5 members including host-emulator)
cargo build --workspace

# Run the in-process emulator end-to-end (no hardware required)
cargo run -p host-emulator

# Host tests
cargo test --workspace

# Firmware example — cd required so .cargo/config.toml is discovered
cd examples/nrf52840-dk && cargo build --release

# Flash to nRF52840-DK (probe-rs download: flashes and exits, probe released)
cd examples/nrf52840-dk && cargo run --release

# CLI tool (excluded from workspace — requires cd)
cd tools/telepath-cli && cargo build
cd tools/telepath-cli && cargo run -- ping
cd tools/telepath-cli && cargo run

# Format check
cargo fmt --all -- --check

# Clippy (warnings are errors)
cargo clippy --workspace -- -D warnings

# All CI checks at once
just ci
```

## Critical Constraints

### `telepath-wire`
- MUST NOT use `alloc` or `std`. The crate is `#![no_std]` and no heap allocation is permitted.
- All types MUST implement `serde::Serialize + serde::Deserialize` with `default-features = false`.
- Lifetime-parameterised types (e.g. `Request<'a>`) MUST borrow from the receive buffer to achieve zero-copy deserialization.

### `examples/nrf52840-dk`
- MUST be built separately; it is excluded from the workspace (`exclude = [...]` in root `Cargo.toml`).
- MUST NOT be added to the workspace `members` list; it has its own `target` directory and Cargo config.
- Cross-compilation REQUIRES `rustup target add thumbv7em-none-eabi`.
- `cargo run --release` invokes `probe-rs download` (flash + exit). The probe is released immediately so `telepath-cli` can attach.

### `examples/host-emulator`
- IS a workspace member (`std` target, no cross-compile). Build with `cargo build --workspace`.
- MUST exercise the full wire path including COBS framing — it is the primary hardware-free regression for `telepath-firmware` and `telepath-host`.
- MUST use only public APIs of the dependent crates; it MUST NOT poke internal state to aid the round-trip.
- CI runs `timeout 30 cargo run -p host-emulator` and grep-asserts the `ping -> 0xDEADBEEF` output on every push.

### `tools/telepath-cli`
- MUST be built separately; it is excluded from the workspace.
- MUST NOT be built with `cargo build -p telepath-cli` from the workspace root (not a workspace member).
- Firmware MUST be flashed (and probe released) before invoking the CLI.

### `telepath-firmware`
- MUST remain `#![no_std]`.
- MUST NOT depend on `std` or `alloc` directly.

## Wire Protocol Rules

| Property | Specification |
|----------|---------------|
| Downstream framing (Host→Target) | COBS; delimiter `0x00`; MCU decoder is a simple `read_until(0x00)` state machine |
| Upstream framing (Target→Host) | COBS in current MVP; `0x00` delimiter. rzCOBS planned for Stage C2 (see [Issue #3](https://github.com/tarotene/telepath/issues/3)) |
| Serialization | postcard (little-endian, varint-compressed) |
| Packet type | 2-valued: `Request` (0x01) / `Response` (0x02); follows ONC RPC RFC 5531 CALL/REPLY model |
| Error representation | `ResponseStatus` field inside `Response`; NOT a separate packet type |
| Discovery CmdID | `0x0000` — RESERVED for Command Discovery Protocol (CDP); follows CoAP Empty / ONC RPC NULL convention |
| Max payload | 256 bytes (`MAX_PAYLOAD_SIZE`) |

## `#[command]` Macro

- Current state: **passthrough stub** — the function is emitted unchanged.
- Planned: generate type-erased shim + `CommandMetadata` static + `linkme` distributed slice registration.
- Changes to the macro MUST NOT break existing callers on stable toolchain.

## Commit and PR Rules

- Follow Conventional Commits: `feat(wire): add CRC field to Request`
- Feature branches MUST be created before any code change.
- PRs MUST reference the corresponding GitHub Issue.
- `examples/nrf52840-dk/` changes SHOULD be a separate commit from workspace changes.

## Toolchain

- Channel: `stable` (pinned in `rust-toolchain.toml`)
- Additional target: `thumbv7em-none-eabi`
- Recommended tools: `just`, `probe-rs`, `cargo-expand` (for macro debugging)

## Git Hooks

After cloning, contributors MUST run:

```
git config core.hooksPath .githooks
```

- `pre-commit` → `just fmt-check` (sub-second; runs on every commit)
- `pre-push` → `just clippy` + `just test` (~30 s; runs before every push)
- `just ci` (fmt-check + clippy + test + emulator) SHOULD be run before opening a PR.
