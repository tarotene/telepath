# Telepath — task runner
# Requires: https://github.com/casey/just

# Build all workspace members (host targets only)
build:
    cargo build --workspace

# Run all workspace tests
test:
    cargo test --workspace

# Check formatting (no changes written)
fmt-check:
    cargo fmt --all -- --check

# Apply formatting
fmt:
    cargo fmt --all

# Run clippy on workspace members only
clippy-workspace:
    cargo clippy --workspace -- -D warnings

# Run clippy on tools/telepath across all relevant feature combinations
clippy-tools:
    cargo clippy --manifest-path tools/telepath/Cargo.toml --all-targets -- -D warnings
    cargo clippy --manifest-path tools/telepath/Cargo.toml --no-default-features --features shell,serial --all-targets -- -D warnings
    cargo clippy --manifest-path tools/telepath/Cargo.toml --no-default-features --features mcp,serial --all-targets -- -D warnings

# Run clippy everywhere (workspace + tools); warnings are treated as errors
clippy: clippy-workspace clippy-tools

# Verify a single commit message file against Conventional Commits (used by .githooks/commit-msg)
# Requires: cargo install --locked cocogitto
commit-check MSG_FILE:
    cog verify --file '{{MSG_FILE}}'

# Build firmware example (cross-compile; requires thumbv7em-none-eabi target)
# Must cd into the example dir so .cargo/config.toml picks up target = "thumbv7em-none-eabi"
firmware-build:
    cd examples/nrf52840-ping && cargo build --release

# Flash firmware to nRF52840-DK and bring the core to run state.
# Two steps: `cargo run` flashes via probe-rs and releases the probe;
# `probe-rs reset` then exits reset-and-halt so the firmware starts and
# `rtt_init!` populates the RTT control block before any host attach.
firmware-flash:
    cd examples/nrf52840-ping && cargo run --release
    probe-rs reset --chip nRF52840_xxAA

# Build telepath CLI (default features: shell + mcp + rtt)
cli-build:
    cd tools/telepath && cargo build

# Run telepath shell with arguments (firmware must already be flashed)
cli *ARGS:
    cd tools/telepath && cargo run -- shell {{ARGS}}

# Local end-to-end smoke: rebuild FW, flash, run `ping` once, assert sentinel.
# Requires nRF52840-DK connected.  Catches wire-format skew between FW and host.
firmware-ping: firmware-flash
    #!/usr/bin/env bash
    set -euo pipefail
    cd tools/telepath && cargo run -- shell --exec ping | tee /dev/stderr | grep -qF "ping -> 0xDEADBEEF"

# Run telepath tests across feature matrices (codec, bridge, MCP server, serial PTY smoke)
mcp-test:
    cd tools/telepath && cargo test
    cd tools/telepath && cargo test --no-default-features --features mcp,serial

# Run telepath mcp server against a flashed nRF52840-DK (firmware must already be flashed)
mcp-run-rtt:
    cd tools/telepath && cargo run -- mcp

# Build everything: workspace + firmware + CLI
check-all: build firmware-build cli-build

# Smoke test host-pty-server end-to-end via serial telepath shell (no hardware required)
host-pty-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p host-pty-server
    cargo build --manifest-path tools/telepath/Cargo.toml --no-default-features --features shell,serial
    cargo run -p host-pty-server > /tmp/host-pty-server.out &
    SERVER_PID=$!
    trap 'kill "$SERVER_PID" 2>/dev/null || true; wait "$SERVER_PID" 2>/dev/null || true' EXIT
    for i in $(seq 1 15); do
        SLAVE=$(grep 'HOST_PTY_SERVER_PATH=' /tmp/host-pty-server.out 2>/dev/null | sed 's/HOST_PTY_SERVER_PATH=//' | head -1 || true)
        if [ -n "$SLAVE" ]; then break; fi
        sleep 1
    done
    if [ -z "$SLAVE" ]; then
        echo "ERROR: host-pty-server did not print PTY path"; exit 1
    fi
    tools/telepath/target/debug/telepath shell --transport serial --port "$SLAVE" --exec ping | tee /dev/stderr | grep -qF "ping -> 0xDEADBEEF"
    tools/telepath/target/debug/telepath shell --transport serial --port "$SLAVE" --exec "add 2 3" | tee /dev/stderr | grep -qF "add -> 5"
    tools/telepath/target/debug/telepath shell --transport serial --port "$SLAVE" --exec "add [2, 3]" | tee /dev/stderr | grep -qF "add -> 5"

# Full CI gate: fmt-check + clippy + test + host-pty smoke + telepath tests
ci: fmt-check clippy test host-pty-smoke mcp-test

# Preview what the next release would update, then restore the working tree.
# Requires: cargo install --locked release-plz
release-preview:
    #!/usr/bin/env bash
    set -euo pipefail
    release-plz update
    git diff
    git restore .

# Bump excluded crates' versions to match the upcoming workspace release.
# Usage: just bump-excluded 0.1.1
# Run as the final commit on the release-plz PR before merge.
bump-excluded VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in tools/telepath/Cargo.toml examples/nrf52840-ping/Cargo.toml; do
        sed 's/^version = "[^"]*"/version = "{{VERSION}}"/' "$f" > "$f.tmp" \
            && mv "$f.tmp" "$f"
    done
