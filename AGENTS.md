# Telepath — Agent Configuration

> Project-specific rules for AI coding agents.
> RFC 2119 keywords (MUST, SHOULD, MAY) indicate requirement strength.

## Workspace Overview

| Crate | Role | Target |
|-------|------|--------|
| `telepath-wire` | Shared wire protocol types | server + client |
| `telepath-macros` | `#[command]` proc-macro | server (build time only) |
| `telepath-server` | Target-side RPC server library | `thumbv7em-none-eabi` |
| `telepath-client` | Host-side RPC client library; `rtt` and `serial` Cargo features select the transport | native (`std`) |
| `examples/host-pty-server` | Host-side server deployment over a PTY pair (hardware-free regression) | native (`std`) |
| `examples/nrf52840-ping` | Reference server deployment on nRF52840-DK | `thumbv7em-none-eabi` |
| `tools/telepath` | Unified CLI: `telepath shell` (interactive REPL) and `telepath mcp` (MCP server); `default = ["shell", "mcp", "rtt"]`, `serial` opt-in | native (`std`) |

## Build Commands

```
# Host workspace (all 5 members including host-pty-server)
cargo build --workspace

# Run host-pty-server (prints slave PTY path; connect telepath shell --transport serial to that path)
cargo run -p host-pty-server

# Full hardware-free smoke via just (spawns server + serial shell, asserts ping)
just host-pty-smoke

# Host tests
cargo test --workspace

# Server example — cd required so .cargo/config.toml is discovered
cd examples/nrf52840-ping && cargo build --release

# Flash to nRF52840-DK (probe-rs download: flashes and exits, probe released)
cd examples/nrf52840-ping && cargo run --release

# Telepath unified CLI (excluded from workspace — requires cd)
# Default build: shell + mcp + rtt
cd tools/telepath && cargo build
cd tools/telepath && cargo run -- shell --exec ping
cd tools/telepath && cargo run -- shell
# Serial build: shell subcommand with serial transport
cd tools/telepath && cargo build --no-default-features --features shell,serial
cd tools/telepath && cargo run --no-default-features --features shell,serial -- shell --transport serial --port /dev/ttyACM0
# MCP server: default build includes mcp subcommand
cd tools/telepath && cargo run -- mcp
cd tools/telepath && cargo test

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
- `cargo run --release` invokes `probe-rs download` (flash + exit). The probe is released immediately so `telepath shell` (with RTT transport) can attach.

### `examples/host-pty-server`
- IS a workspace member (`std` target, no cross-compile). Build with `cargo build --workspace`.
- MUST exercise the full wire path including COBS framing via a real PTY transport — it is the primary hardware-free regression for `telepath-server` and the serial path of `telepath-client`.
- MUST use only public APIs of the dependent crates; it MUST NOT poke internal state to aid the round-trip.
- On startup, prints `HOST_PTY_SERVER_PATH=<path>` to stdout then flushes; the test harness reads this to obtain the slave device path.
- CI spawns `host-pty-server` in background, reads the slave path, runs `telepath shell --transport serial --port <path> --exec ping`, and grep-asserts `ping -> 0xDEADBEEF`.

### `tools/telepath`
- MUST be built separately; it is excluded from the workspace (`exclude = [...]` in root `Cargo.toml`).
- MUST NOT be built with `cargo build -p telepath` from the workspace root (not a workspace member).
- Pure conversion modules (`codec/schema_to_json`, `codec/json_to_postcard`, `codec/postcard_to_json`) MUST remain side-effect free and sync; async lives only in `mcp/server.rs`.
- All MCP subcommand logging MUST go to `stderr`; `stdout` is reserved for the MCP JSON-RPC stream.
- Server MUST be flashed (and probe released) before invoking the shell subcommand with RTT transport.

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
- Wire argument types: any `T: Serialize + DeserializeOwned + postcard_schema::Schema` (owned, no references)
- `#[resource]`-annotated arguments: `&T` or `&mut T` where `T: 'static` — injected from the server's `ResourceRegistry`, not deserialized from the wire
- Wire and resource arguments may appear in any order
- Return type: any `T: Serialize + postcard_schema::Schema` (owned, no references); `()` means "no payload"

Rejected at compile time (`syn::Error`):
- `async fn`, `unsafe fn`
- Generic parameters and `where` clauses
- `&T` / `&mut T` argument WITHOUT `#[resource]` attribute
- `&T` / `&mut T` return type
- Methods (`fn foo(&self, …)`)
- Non-identifier argument patterns (e.g. tuple destructuring)
- Duplicate `#[resource]` types (each resource type may appear at most once)

Wire encoding:
- Only non-`#[resource]` arguments are serialized; resource arguments are server-side only
- Args serialized as a postcard tuple: `()` (0-arg), `(T,)` (1-arg), `(T1, T2, …)` (N-arg)
- Return value serialized standalone (no wrapper tuple)
- CmdID derived deterministically from `(name, args_type_str, ret_type_str)` using wire args only — adding or removing a `#[resource]` argument does NOT change the wire CmdID

### Generated items

For each `#[command] fn foo(…) -> R`, the macro emits:
- `__telepath_shim_foo` — type-erased shim: postcard-deserializes args, calls `foo`, serializes return value
- `__telepath_args_schema_foo` / `__telepath_ret_schema_foo` — write postcard-encoded `NamedType` schema bytes
- `pub const __TELEPATH_CMD_FOO: CommandMetadata`
- `#[linkme::distributed_slice]` static `__TELEPATH_REG_FOO` for zero-cost link-time registration

Changes to the macro MUST NOT break existing callers on stable toolchain.

### Peripheral Access

`#[resource]` is the recommended — and idiomatic — mechanism for giving
`#[command]` functions access to peripherals and other global mutable state.
Prefer it for all new code.

```rust
// 1. Newtype wrapper — gives each peripheral a unique TypeId.
//    HAL types are often lifetime-parametric (Output<'d>), so erasing the
//    lifetime via transmute is necessary.  The transmute is sound when the
//    peripheral token is a 'static ZST that owns no borrowed data.
pub struct Led1(pub Output<'static>);
impl Led1 {
    pub fn new(pin: Output<'_>) -> Self {
        // SAFETY: Output<'d> stores AnyPin by value; 'd is PhantomData only.
        Self(unsafe { core::mem::transmute::<Output<'_>, Output<'static>>(pin) })
    }
}

// 2. Declare resource parameters with #[resource]; wire args appear in any order.
#[command]
fn led_set(#[resource] led1: &mut Led1, id: u8, on: bool) -> bool {
    // led1 is injected by the server; id and on are decoded from the wire.
    // …
}

// 3. Register resources on the server builder — one .resource() call per type.
let mut server = TelepathServer::<MyTransport, 512>::new(transport, telepath_server::commands())
    .resource(Led1::new(Output::new(pin, Level::High, OutputDrive::Standard)));
```

See [`examples/nrf52840-ping/src/main.rs`](examples/nrf52840-ping/src/main.rs) for a
complete multi-peripheral example.

**Runtime invariants:**

- Each resource type may appear **at most once** in the registry; registering a
  second value of the same type panics at runtime (fail-fast to prevent silent
  shadowing). Duplicate `#[resource]` arguments within a single `#[command]`
  signature are additionally rejected at compile time by the proc-macro.
- `T: 'static` is required — HAL types with lifetime parameters must be newtype-wrapped
  and `transmute`d to `'static` (the soundness obligation rests with the crate author).
- Resource arguments are **wire-transparent**: they are not serialized into the wire
  payload and do not affect the `cmd_id` calculation.  Adding or removing a
  `#[resource]` argument is therefore **not a breaking wire change**.

**Legacy pattern:** If `#[resource]` cannot be adopted (e.g. the peripheral is already
shared via a `static Mutex<RefCell<Option<T>>>` elsewhere in the firmware), that pattern
remains valid and is equally wire-transparent — `#[command]` functions may close over
global statics directly.  New code SHOULD prefer `#[resource]`.

## Commit and PR Rules

- Follow Conventional Commits: `feat(wire): add CRC field to Request`
- Feature branches MUST be created before any code change.
- PRs MUST reference the corresponding GitHub Issue.
- `examples/nrf52840-ping/` changes SHOULD be a separate commit from workspace changes.
- PRs that touch any of the following SHOULD be smoke-tested with `just firmware-ping`
  against a connected nRF52840-DK before requesting review, and the result recorded in
  the PR description's Test plan section:
  - `telepath-wire/`, `telepath-macros/`, `telepath-server/`, `telepath-client/`
  - `tools/telepath/`
  - `examples/nrf52840-ping/`

  This catches FW/host wire-format skew that `just ci` alone cannot detect without hardware.

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
- `just ci` (fmt-check + clippy + test + host-pty-smoke + mcp-test) SHOULD be run before opening a PR.
- `just firmware-ping` SHOULD additionally be run when the PR touches wire / macros /
  server / client / shell / nrf52840-ping (see "Commit and PR Rules" above).
