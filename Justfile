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

# Flash firmware to nRF52840-DK via probe-rs
firmware-flash:
    cd examples/nrf52840-dk && cargo run --release

# Build everything: workspace + firmware
check-all: build firmware-build

# Full CI gate: fmt-check + clippy + test
ci: fmt-check clippy test
