# telepath-macros

Proc-macro crate providing the `#[command]` attribute that registers a plain Rust
function as a Telepath RPC command — no hand-written dispatch boilerplate required.

This crate is a **build-time only** dependency of `telepath-server`. It does not
appear in device firmware at runtime.

## Usage

```rust
use telepath_server::command;

#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

#[command]
fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

The `#[command]` attribute is re-exported from `telepath-server`; no direct
dependency on `telepath-macros` is required in downstream crates.

## Generated items

For each `#[command] fn foo(…) -> R` the macro emits:

| Item | Description |
|------|-------------|
| `fn __telepath_shim_foo(input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError>` | Type-erased shim: deserializes args via postcard, calls `foo`, serializes return value |
| `fn __telepath_args_schema_foo(out: &mut [u8]) -> Result<usize, ()>` | Writes postcard-encoded `NamedType` schema for the argument tuple |
| `fn __telepath_ret_schema_foo(out: &mut [u8]) -> Result<usize, ()>` | Writes postcard-encoded `NamedType` schema for the return type |
| `pub const __TELEPATH_CMD_FOO: CommandMetadata` | Const holding name, `cmd_id`, and function pointers |
| `static __TELEPATH_REG_FOO: CommandMetadata` | `#[linkme::distributed_slice]` registration — zero-cost link-time registration in `TELEPATH_COMMANDS` |

The original function body is preserved unchanged and remains directly callable.

## Signature contract

**Allowed:**

- Free functions only (no `self` receiver)
- Any number of positional arguments with simple identifier patterns
- Argument types: any `T: Serialize + DeserializeOwned + postcard_schema::Schema` (owned, no references)
- Return type: any `T: Serialize + postcard_schema::Schema` (owned, no references); `()` means "no payload"

**Rejected at compile time:**

- `async fn`, `unsafe fn`
- Generic parameters or `where` clauses
- `&T` / `&mut T` arguments or return types
- Methods (`fn foo(&self, …)`)
- Non-identifier argument patterns (e.g. tuple destructuring `(a, b): (i32, i32)`)

## Wire encoding

- **Args:** serialized as a postcard tuple — `()` (zero args), `(T,)` (one arg), `(T1, T2, …)` (N args)
- **Return value:** serialized standalone (no wrapper tuple)
- **`cmd_id`:** derived deterministically from `(name, args_type_str, ret_type_str)` via FNV-1a 16-bit — renaming a function or changing a type signature is a **breaking wire change**
- Reserved `cmd_id = 0x0000` is avoided by deterministic salt rehashing in `derive_cmd_id`; the discovery ID is never emitted by user commands

## Build

```bash
# Built automatically as part of the workspace
cargo build --workspace

# Inspect macro expansion in a consumer crate (requires cargo-expand)
cd telepath-server && cargo expand --test macro_smoke
```

`telepath-macros` is a workspace member targeting the native host (proc-macro crates
run on the build host, not the embedded target). No cross-compilation is needed.

## Toolchain

Stable Rust (pinned in `rust-toolchain.toml` at the repo root).
Changes MUST NOT break existing callers on the pinned stable channel.
