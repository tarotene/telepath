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
    cd examples/nrf52840-dk && cargo build --release

# Flash firmware to nRF52840-DK (downloads and exits; probe is released)
firmware-flash:
    cd examples/nrf52840-dk && cargo run --release

# Build telepath-cli
cli-build:
    cd tools/telepath-cli && cargo build

# Run telepath-cli with arguments (firmware must already be flashed)
cli *ARGS:
    cd tools/telepath-cli && cargo run -- {{ARGS}}

# End-to-end smoke test: flash then ping
firmware-ping: firmware-flash
    cd tools/telepath-cli && cargo run -- ping

# Build everything: workspace + firmware + CLI
check-all: build firmware-build cli-build

# Run the in-process emulator end-to-end (no hardware required)
emulator:
    cargo run -p host-emulator

# Full CI gate: fmt-check + clippy + test + emulator smoke
ci: fmt-check clippy test emulator
