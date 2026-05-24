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

# Run clippy; warnings are treated as errors
clippy:
    cargo clippy --workspace -- -D warnings

# Build firmware example (cross-compile; requires thumbv7em-none-eabi target)
# Must cd into the example dir so .cargo/config.toml picks up target = "thumbv7em-none-eabi"
firmware-build:
    cd examples/nrf52840-ping && cargo build --release

# Flash firmware to nRF52840-DK (downloads and exits; probe is released)
firmware-flash:
    cd examples/nrf52840-ping && cargo run --release

# Build telepath-shell
cli-build:
    cd tools/telepath-shell && cargo build

# Run telepath-shell with arguments (firmware must already be flashed)
cli *ARGS:
    cd tools/telepath-shell && cargo run -- {{ARGS}}

# Local end-to-end smoke: rebuild FW, flash, run `ping` once, assert sentinel.
# Requires nRF52840-DK connected.  Catches wire-format skew between FW and host.
firmware-ping: firmware-flash
    cd tools/telepath-shell && cargo run -- --exec ping | tee /dev/stderr | grep -qF "ping -> 0xDEADBEEF"

# Build telepath-mcp-server
mcp-build:
    cd tools/telepath-mcp-server && cargo build

# Run telepath-mcp-server tests
mcp-test:
    cd tools/telepath-mcp-server && cargo test

# Run telepath-mcp-server against a flashed nRF52840-DK (firmware must already be flashed)
mcp-run-rtt:
    cd tools/telepath-mcp-server && cargo run

# Build everything: workspace + firmware + CLI + MCP server
check-all: build firmware-build cli-build mcp-build

# Smoke test host-pty-server end-to-end via serial telepath-shell (no hardware required)
host-pty-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p host-pty-server
    cargo build --manifest-path tools/telepath-shell/Cargo.toml --no-default-features --features serial
    cargo run -p host-pty-server > /tmp/host-pty-server.out &
    SERVER_PID=$!
    for i in $(seq 1 15); do
        SLAVE=$(grep 'HOST_PTY_SERVER_PATH=' /tmp/host-pty-server.out 2>/dev/null | sed 's/HOST_PTY_SERVER_PATH=//' | head -1 || true)
        if [ -n "$SLAVE" ]; then break; fi
        sleep 1
    done
    if [ -z "$SLAVE" ]; then
        echo "ERROR: host-pty-server did not print PTY path"; kill $SERVER_PID; exit 1
    fi
    tools/telepath-shell/target/debug/telepath-shell --port "$SLAVE" --exec ping | tee /dev/stderr | grep -qF "ping -> 0xDEADBEEF"
    kill $SERVER_PID 2>/dev/null || true

# Full CI gate: fmt-check + clippy + test + host-pty smoke + mcp-test
ci: fmt-check clippy test host-pty-smoke mcp-test
