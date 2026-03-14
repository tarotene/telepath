//! Telepath example for nRF52840-DK.
//!
//! Demonstrates a minimal embassy application that initialises a
//! [`TelepathServer`] and spins waiting for RPC requests over RTT.
//!
//! # Building
//!
//! ```
//! cargo build --manifest-path examples/nrf52840-dk/Cargo.toml --release
//! ```
//!
//! # Flashing
//!
//! ```
//! cargo run --manifest-path examples/nrf52840-dk/Cargo.toml --release
//! ```
#![no_std]
#![no_main]

use defmt::info;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use panic_probe as _;
use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};

// ---------------------------------------------------------------------------
// Example command: toggle LED 1
// ---------------------------------------------------------------------------

fn led_toggle_shim(_input: &[u8], _output: &mut [u8]) -> Result<usize, DispatchError> {
    // Actual LED toggling happens via the shared mutable state in a real app.
    // This stub demonstrates the shim signature.
    Ok(0)
}

static COMMANDS: [CommandMetadata; 1] = [CommandMetadata {
    name: "led_toggle",
    id: 0x0001,
    invoke: led_toggle_shim,
}];

// ---------------------------------------------------------------------------
// Embassy main
// ---------------------------------------------------------------------------

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    // LED 1 on nRF52840-DK is P0.13, active low.
    let mut led = Output::new(p.P0_13, Level::High, OutputDrive::Standard);

    info!("Telepath nRF52840-DK example started");

    // In a full implementation, pass an RTT or UART transport here.
    // For now we use a zero-sized placeholder.
    struct NoopTransport;
    let mut server = TelepathServer::<NoopTransport, 512>::new(NoopTransport, &COMMANDS);

    let mut tick: u32 = 0;
    loop {
        // Blink LED to show liveness while waiting for RPC traffic.
        if tick % 2 == 0 {
            led.set_low();
        } else {
            led.set_high();
        }
        tick = tick.wrapping_add(1);

        // TODO: feed bytes from transport into server.dispatch() once
        // the framing layer is wired up.
        let _ = &mut server;

        // Simple spin-delay; replace with embassy_time::Timer in production.
        cortex_m::asm::delay(8_000_000); // ~1 s at 8 MHz
    }
}
