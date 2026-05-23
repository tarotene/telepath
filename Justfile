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

# End-to-end smoke test: flash then ping
firmware-ping: firmware-flash
    cd tools/telepath-shell && cargo run -- ping

# Build telepath-mcp-server
mcp-build:
    cd tools/telepath-mcp-server && cargo build

# Run telepath-mcp-server tests
mcp-test:
    cd tools/telepath-mcp-server && cargo test

# Install npm dependencies for MCP E2E tests (run once; browser not required)
mcp-e2e-install:
    cd tools/telepath-mcp-server/e2e && npm install

# Run Inspector E2E tests (requires cargo build first)
mcp-e2e: mcp-build
    cd tools/telepath-mcp-server/e2e && npm test

# Build everything: workspace + firmware + CLI + MCP server
check-all: build firmware-build cli-build mcp-build

# Run the in-process emulator end-to-end (no hardware required)
emulator:
    cargo run -p loopback-demo

# Full CI gate: fmt-check + clippy + test + emulator smoke
ci: fmt-check clippy test emulator
