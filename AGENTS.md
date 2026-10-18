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
| `tools/telepath` | Unified CLI: `telepath shell` (interactive REPL) and `telepath mcp` (MCP server); feature flags are listed in the crate README | native (`std`) |

## Documentation Source of Truth

This repository uses a layered documentation model. Each topic has a single
canonical home (Source of Truth, SoT); all other documents link to it instead
of duplicating content. When you find the same content in two places, keep the
SoT copy and replace the other with a link.

**Layering rules:**
- `AGENTS.md` = SoT for build commands, workspace structure, CI gate rules,
  MSRV policy, git hooks, and workflow rules
- Crate-level `README.md` = SoT for that crate's API surface, usage examples,
  feature flags, and Limitations
- Root `README.md` = user-facing narrative; quickstart lives here, other
  topics link to AGENTS or crate READMEs
- `docs/` = SoT for deep operating documents (release runbook, MCP
  integration details)
- The workflow and composite-action files carry their own purpose and trigger
  description in their header comments; AGENTS.md states only the rules, never
  an inventory of jobs or workflows

The sections below that depend on one of these homes link to it at the point
of use.

**Limitations sections**: each crate README's `## Limitations` MUST reference
an open issue and MUST be removed in the same PR that implements the feature
(see README Limitations Lifecycle in the user-global AGENTS.md).

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
- The `codec` module (schema and JSON/postcard conversions) MUST remain side-effect free and sync; async lives only in the `mcp` module.
- All MCP subcommand logging MUST go to `stderr`; `stdout` is reserved for the MCP JSON-RPC stream.
- Server MUST be flashed (and probe released) before invoking the shell subcommand with RTT transport.

### `telepath-server`
- MUST remain `#![no_std]`.
- MUST NOT depend on `std` or `alloc` directly.

## Wire Protocol Rules

| Property | Specification |
|----------|---------------|
| Downstream framing (Host→Target) | COBS; delimiter `0x00`; MCU decoder is a simple `read_until(0x00)` state machine |
| Upstream framing (Target→Host) | rzCOBS; `0x00` delimiter |
| Serialization | postcard (little-endian, varint-compressed) |
| Packet type | 2-valued: `Request` (0x01) / `Response` (0x02); follows ONC RPC RFC 5531 CALL/REPLY model |
| Error representation | `ResponseStatus` field inside `Response`; NOT a separate packet type |
| Discovery CmdID | `0x0000` — RESERVED for Command Discovery Protocol (CDP); follows CoAP Empty / ONC RPC NULL convention |
| Max payload | 256 bytes (`MAX_PAYLOAD_SIZE`) |

## `#[command]` Macro

`#[command]` accepts plain free functions only. Wire args must be
`T: Serialize + DeserializeOwned + postcard_schema::Schema` (owned); `&T`/`&mut T`
requires the `#[resource]` attribute. `async`/`unsafe`/generics/methods are rejected
at compile time. CmdID is derived from wire args only — adding or removing a
`#[resource]` argument is **not** a breaking wire change.

For the full signature contract, generated items, and wire encoding details see
[telepath-macros/README.md § Signature contract](telepath-macros/README.md#signature-contract).

Changes to the macro MUST NOT break existing callers on stable toolchain.

### Peripheral Access

`#[resource]` is the recommended — and idiomatic — mechanism for giving
`#[command]` functions access to peripherals and other global mutable state.
Prefer it for all new code.

Worked example: [examples/nrf52840-ping/README.md § Resource injection](examples/nrf52840-ping/README.md#resource-injection)
and [`examples/nrf52840-ping/src/main.rs`](examples/nrf52840-ping/src/main.rs).

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
  Enforced locally by the `commit-msg` hook via cocogitto (`cog verify`).
- Feature branches MUST be created before any code change.
- PRs MUST reference the corresponding GitHub Issue.
- Changes to the firmware example SHOULD be a separate commit from workspace changes.
- PRs that touch any crate in § Workspace Overview except the hardware-free
  `host-pty-server` example SHOULD be smoke-tested with `just firmware-ping`
  against a connected nRF52840-DK before requesting review, and the result recorded in
  the PR description's Test plan section. This catches FW/host wire-format skew that
  `just ci` alone cannot detect without hardware.

### How releases work

**Humans MUST NOT create git tags or GitHub Releases manually.**
Everything is driven by release-plz via GitHub Actions; the jobs and what each
one does are described in the header comment of the release-plz workflow.

#### Normal release cycle

1. Merge any PR whose commits include `feat:`, `fix:`, `perf:`, or `refactor:` prefixes.
2. release-plz opens a `chore: release vX.Y.Z` PR automatically (version bump,
   changelog entry, `release` label) and bumps the workspace-excluded crates onto
   the same PR branch. **No manual action is required.**
3. Review and merge the release PR.
4. release-plz creates a **draft** GitHub Release for the unified workspace
   version; the binaries workflow attaches the prebuilt binaries and then
   publishes it. The release stays a draft if that step fails.

Recovery, retrigger, version override, GitHub App token setup, and Trusted
Publishing (including the one-time manual bootstrap MUST for a new publishable
crate) are in [`docs/releasing.md`](docs/releasing.md).

#### Release scheduling model (hybrid)

| Change type | Version | When to merge the Release PR |
|-------------|---------|------------------------------|
| Bug fix / non-breaking improvement | Patch (`v0.X.(Y+1)`) | As soon as it is ready |
| Feature addition (non-breaking) | Minor (`v0.(X+1).0`) | When the target Milestone is 100% closed |
| Breaking change | Minor (`v0.(X+1).0`, pre-1.0) | **Always bundle into a Minor Milestone** |

**Wire-protocol breaking changes** (e.g. rzCOBS framing) require firmware and host to update
simultaneously; never release them in isolation.

A scheduled workflow posts a weekly comment on any Release PR that has been open for more
than 7 days. If no Release PR exists despite qualifying commits on `main`, the release-plz
workflow failed — see the retriggering section of the release runbook.

## Toolchain

- Channel: `stable` (pinned in `rust-toolchain.toml`)
- MSRV: `1.88` (declared via `rust-version` in all `Cargo.toml` files and `constraints.rust` in `renovate.json` — note: Renovate's `semver` versioning rejects ranges, so use a single `X.Y.Z` literal, e.g. `1.88.0`)
- When bumping MSRV, update `rust-version` in all manifests **and** `constraints.rust` in `renovate.json` (`X.Y.Z` literal, NOT a range) in the same PR.
  Renovate will NOT auto-propose this change (the `renovate-config`/`rust` packageRule sets `enabled: false` — see § Dependency Management); the update MUST be made manually as part of the `feat(toolchain)!` commit.
- Additional target: `thumbv7em-none-eabi`
- Recommended tools: `just`, `probe-rs`, `cargo-expand` (for macro debugging), `cocogitto` (commit-msg validation)

### MSRV policy

The MSRV is verified in CI (`msrv` job, `dtolnay/rust-toolchain@1.88.0`).
Bumping the MSRV is a MINOR change for pre-1.0 releases and MUST use the
commit convention `feat(toolchain)!: bump MSRV to 1.XX`.

### Required CI gates

The repository uses **three layered Rulesets** targeting `~DEFAULT_BRANCH` (`main`):
`Security` (deletion and force-push protection), `Quality` (required status checks,
signed commits, linear history), and `Workflow` (squash-only PRs with thread
resolution, Copilot code review). `Security` and `Quality` have no bypass actors.
When multiple rulesets target the same branch, GitHub enforces the **most restrictive**
combination; the split ensures that `Security` and `Quality` remain absolute
regardless of `Workflow` configuration.

The rulesets are declared in `.github/rulesets/*.json`; that declaration, not this
document, is the list of rules and required status checks. Ruleset updates MUST be
applied through the API (look up the live ids with `gh api repos/tarotene/telepath/rulesets`)
so changes are auditable.

Only two status checks are required (ADR-591, tarotene/dotfiles): `CI passed` (the
aggregate job in `ci.yml`, which `needs:` every other PR-gate job) and `PR title`.
The individual jobs are not required individually; they gate merging through
`CI passed`.

**Decision criteria for promoting a job to Required:**
- Correctness / Style / Policy gates → SHOULD be required
- Hardware-dependent jobs (e.g. `firmware-ping`) → NOT required without a self-hosted runner
- Experimental or known-flaky jobs → NOT required until stable across ≥5 consecutive PRs
- Release-only jobs cannot gate PRs and are NOT required

### CI tool installation policy

When adding new tooling to CI workflows, choose the delivery mechanism in this order:

1. **Dedicated setup-action** — e.g. `rui314/setup-mold`, `dtolnay/rust-toolchain`. Fastest; no compile step.
2. **`taiki-e/install-action`** — for tools listed in its manifest (e.g. `just`, `nextest`). Pre-built binary download.
3. **`cargo-binstall`** — for crates with published binaries not covered above.
4. **`cargo install --locked` from source** — last resort only; adds minutes of compile time to every run.

`cargo install` from source MUST NOT be used in CI unless the tool has no binary distribution.

### CI workflow file layout

All PR-gate jobs live in the single CI workflow; its header comment explains the
aggregate-job model. `ci-passed` MUST be updated (`needs:`) whenever a job is added
or removed. The PR-title check is a separate workflow. The release and maintenance
workflows are not PR gates; each carries its purpose and triggers in its own header
comment.

Common setup (toolchain, libudev, just, rust-cache) lives in the `rust-setup`
composite action under `.github/actions/`. Modify it to apply changes uniformly
across all workflows.

### Path-filtering and job skip strategy

Each CI workflow contains a `Detect relevant changes` step that runs `git diff`
between the **merge-base** of the PR base SHA and HEAD. If no relevant files changed,
all subsequent steps are skipped and the workflow still exits **successfully** — required
status checks remain satisfied because GHA reports a job with all steps skipped as success.

No external path-filter action is used; `permissions: contents: read` is sufficient.
`actions/checkout@v4` runs with `fetch-depth: 0` (full history) to guarantee that
`git merge-base` can resolve the common ancestor. Using the merge-base instead of a raw
two-dot diff prevents false positives when main advances after a PR branches off — only
commits that belong to the PR contribute to the changed-file list.
`git fetch origin "$BASE" --depth=1` in the guard ensures the base commit object is
present (needed for fork PRs where the base ref may not be fetched automatically).
If the merge-base or base SHA is unavailable (new branch / zero SHA / fetch error),
the guard defaults to `run=true` (**safe-by-default** principle — prefer false positives
over silently skipping valid checks).

## Lockfile Policy

- `Cargo.lock` is committed for every manifest group: the workspace root and each
  workspace-excluded crate.
- Rationale: (1) current Cargo FAQ defaults to committing; (2) the CLI and the firmware
  example are binary crates where reproducible builds matter; (3) firmware requires
  deterministic binaries.
- Downstream consumers of the library crates do **not** use this lockfile for dependency
  resolution — that is standard Cargo behaviour.
- Workspace-excluded crates carry an empty `[workspace]` table to make them
  self-contained workspaces, stopping Cargo's upward traversal at their own manifest.

## Dependency Management

- Renovate's `renovate.json` is the source of truth for update schedule, grouping, labels,
  and per-package rules; each `packageRule` carries a `description` explaining why it exists.
  Do not restate its values here.
- Whether a Renovate PR may be merged without human review is decided by that
  configuration (and the preset it extends), not by this document.
- `rangeStrategy: "auto"` (= `update-lockfile`) keeps manifest semver ranges stable; only the lockfile is bumped.
- `probe-rs` is intentionally pinned to the `0.31.x` line. To lift the pin, remove its
  `dependencyDashboardApproval` packageRule in `renovate.json` and widen the version
  requirement in every manifest that depends on it.
- Embedded HAL updates (`embassy-*`, `nrf-pac`, `cortex-m-rt`, etc.) carry the `needs-smoke-test` label.
  Run `just firmware-ping` on a connected nRF52840-DK and record the result in the PR before merging.
- The `rust-toolchain` action's `@stable` ref is excluded from Renovate (channel reference, not a version tag).
- `constraints.rust` is the MSRV pin. Renovate MUST NOT bump it: a packageRule disables
  that update because Renovate would otherwise resolve it through an unrelated Docker
  datasource and the value would drift silently. MSRV changes are deliberate
  `feat(toolchain)!` edits only (see § Toolchain for the full update checklist).
- The Dependency Dashboard Issue lists all suppressed updates for visibility.

## Git Hooks

After cloning, contributors MUST run:

```
git config --local core.hooksPath .githooks
```

- `commit-msg` → `cog verify` (instant; runs on every commit message)
- `pre-commit` → `just fmt-check` (sub-second; runs on every commit)
- `pre-push` → `just clippy` + `just test` (~30 s; runs before every push)
- `just ci` (fmt-check + clippy + test + host-pty-smoke + mcp-test) SHOULD be run before opening a PR.
- `just firmware-ping` SHOULD additionally be run when the PR touches wire / macros /
  server / client / shell / nrf52840-ping (see "Commit and PR Rules" above).
