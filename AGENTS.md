# Telepath — Agent Configuration

> Project-specific rules for AI coding agents.
> RFC 2119 keywords (MUST, SHOULD, MAY) indicate requirement strength.

## Workspace Overview

| Crate | Role | Target |
|-------|------|--------|
| `telepath-wire` | Shared wire protocol types | server + client |
| `telepath-macros` | `#[command]` proc-macro | server (build time only) |
| `telepath-server` | Target-side RPC server library | `thumbv7em-none-eabi` |
| `telepath-client` | Host-side RPC client library | native (`std`) |
| `examples/loopback-demo` | In-process server+client loopback | native (`std`) |
| `examples/nrf52840-ping` | Reference server deployment on nRF52840-DK | `thumbv7em-none-eabi` |
| `tools/telepath-shell` | Interactive shell for Telepath servers | native (`std`) |

## Build Commands

```
# Host workspace (all 5 members including loopback-demo)
cargo build --workspace

# Run the in-process loopback end-to-end (no hardware required)
cargo run -p loopback-demo

# Host tests
cargo test --workspace

# Server example — cd required so .cargo/config.toml is discovered
cd examples/nrf52840-ping && cargo build --release

# Flash to nRF52840-DK (probe-rs download: flashes and exits, probe released)
cd examples/nrf52840-ping && cargo run --release

# Shell tool (excluded from workspace — requires cd)
cd tools/telepath-shell && cargo build
cd tools/telepath-shell && cargo run -- ping
cd tools/telepath-shell && cargo run

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

### `examples/nrf52840-ping`
- MUST be built separately; it is excluded from the workspace (`exclude = [...]` in root `Cargo.toml`).
- MUST NOT be added to the workspace `members` list; it has its own `target` directory and Cargo config.
- Cross-compilation REQUIRES `rustup target add thumbv7em-none-eabi`.
- `cargo run --release` invokes `probe-rs download` (flash + exit). The probe is released immediately so `telepath-shell` can attach.

### `examples/loopback-demo`
- IS a workspace member (`std` target, no cross-compile). Build with `cargo build --workspace`.
- MUST exercise the full wire path including COBS framing — it is the primary hardware-free regression for `telepath-server` and `telepath-client`.
- MUST use only public APIs of the dependent crates; it MUST NOT poke internal state to aid the round-trip.
- CI runs `timeout 30 cargo run -p loopback-demo` and grep-asserts the `ping -> 0xDEADBEEF` output on every push.

### `tools/telepath-shell`
- MUST be built separately; it is excluded from the workspace.
- MUST NOT be built with `cargo build -p telepath-shell` from the workspace root (not a workspace member).
- Server MUST be flashed (and probe released) before invoking the shell.

### `telepath-server`
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

### Signature contract

`#[command]` accepts a plain free function with the following constraints.

Allowed:
- Free function only (no `self` or methods)
- Any number of positional arguments (simple identifier patterns only)
- Argument types: any `T: Serialize + DeserializeOwned + postcard_schema::Schema` (owned, no references)
- Return type: any `T: Serialize + postcard_schema::Schema` (owned, no references); `()` means "no payload"

Rejected at compile time (`syn::Error`):
- `async fn`, `unsafe fn`
- Generic parameters and `where` clauses
- `&T` / `&mut T` argument or return type
- Methods (`fn foo(&self, …)`)
- Non-identifier argument patterns (e.g. tuple destructuring)

Wire encoding:
- Args serialized as a postcard tuple: `()` (0-arg), `(T,)` (1-arg), `(T1, T2, …)` (N-arg)
- Return value serialized standalone (no wrapper tuple)
- CmdID derived deterministically from `(name, args_type_str, ret_type_str)` — renaming a function or changing a type is a breaking wire change

### Generated items

For each `#[command] fn foo(…) -> R`, the macro emits:
- `__telepath_shim_foo` — type-erased shim: postcard-deserializes args, calls `foo`, serializes return value
- `__telepath_args_schema_foo` / `__telepath_ret_schema_foo` — write postcard-encoded `NamedType` schema bytes
- `pub const __TELEPATH_CMD_FOO: CommandMetadata`
- `#[linkme::distributed_slice]` static `__TELEPATH_REG_FOO` for zero-cost link-time registration

Changes to the macro MUST NOT break existing callers on stable toolchain.

## Commit and PR Rules

- Follow Conventional Commits: `feat(wire): add CRC field to Request`
- Feature branches MUST be created before any code change.
- PRs MUST reference the corresponding GitHub Issue.
- `examples/nrf52840-ping/` changes SHOULD be a separate commit from workspace changes.

## Toolchain

- Channel: `stable` (pinned in `rust-toolchain.toml`)
- Additional target: `thumbv7em-none-eabi`
- Recommended tools: `just`, `probe-rs`, `cargo-expand` (for macro debugging)

## Git Hooks

After cloning, contributors MUST run:

```
git config --local core.hooksPath .githooks
```

- `pre-commit` → `just fmt-check` (sub-second; runs on every commit)
- `pre-push` → `just clippy` + `just test` (~30 s; runs before every push)
- `just ci` (fmt-check + clippy + test + emulator) SHOULD be run before opening a PR.
