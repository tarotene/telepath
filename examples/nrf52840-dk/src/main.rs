//! Telepath RTT example for nRF52840-DK.
//!
//! Demonstrates a minimal Embassy application that:
//! 1. Initialises RTT with two channels (channel 0 for debug prints, channel 1
//!    for Telepath RPC traffic).
//! 2. Registers the `ping` command (CmdID `0x0001`), which returns
//!    `0xDEADBEEF: u32`.
//! 3. Spins in a loop calling `server.poll()` to handle incoming RPC requests.
//!
//! # Building
//!
//! ```
//! cd examples/nrf52840-dk
//! cargo build --release
//! ```
//!
//! # Flashing
//!
//! ```
//! cd examples/nrf52840-dk
//! cargo run --release
//! ```
#![no_std]
#![no_main]

mod rtt_transport;

use embassy_executor::Spawner;
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use panic_halt as _;
use rtt_target::{rprintln, rtt_init};
use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};

use rtt_transport::RttTransport;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Ping command: no arguments, returns `0xDEADBEEF: u32`.
///
/// CmdID `0x0001` — the simplest possible sanity-check command.
fn ping_shim(_input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError> {
    let val: u32 = 0xDEAD_BEEF;
    let s = postcard::to_slice(&val, output).map_err(|_| DispatchError::SerializeError)?;
    Ok(s.len())
}

static COMMANDS: [CommandMetadata; 1] = [CommandMetadata {
    name: "ping",
    id: 0x0001,
    invoke: ping_shim,
}];

// ---------------------------------------------------------------------------
// Embassy main
// ---------------------------------------------------------------------------

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    // Initialise RTT.
    // Up 0 / Down 0: reserved (debug prints up, placeholder down)
    // Up 1 / Down 1: Telepath RPC transport
    //
    // Down channel indices MUST be contiguous starting at 0; omitting index 0
    // would cause rtt_init! to write to channels.down[1] in a 1-element array
    // (out-of-bounds UB) and advertise num_down_channels=1 so the host's
    // down_channel(1) call returns None.
    let channels = rtt_init! {
        up: {
            0: { size: 1024, name: "print" }
            1: { size: 512,  name: "telepath" }
        }
        down: {
            0: { size: 1,   name: "reserved" }
            1: { size: 512, name: "telepath" }
        }
        // Pin the control block to .segger_rtt so _SEGGER_RTT lands at
        // 0x20000000 (RTT_CTRL in memory.x). The host CLI attaches there
        // directly via ScanRegion::Exact — see tools/telepath-cli/src/rtt_transport.rs.
        section_cb: ".segger_rtt"
    };
    rtt_target::set_print_channel(channels.up.0);
    let rtt_transport = RttTransport::new(channels.up.1, channels.down.1);

    rprintln!("Telepath nRF52840-DK started");

    // LED 1 on nRF52840-DK is P0.13, active low.
    let mut led = Output::new(p.P0_13, Level::High, OutputDrive::Standard);

    let mut server = TelepathServer::<RttTransport, 512>::new(rtt_transport, &COMMANDS);

    let mut tick: u32 = 0;
    loop {
        // Process any pending RPC requests.
        server.poll();

        // Blink LED to show liveness; short delay keeps the poll loop responsive.
        if tick.is_multiple_of(20) {
            led.set_low();
        } else if tick % 20 == 10 {
            led.set_high();
        }
        tick = tick.wrapping_add(1);

        // NOTE: cortex_m::asm::delay is a busy-wait that blocks the Embassy
        // executor. This is acceptable here because this is a single-task
        // demo — server.poll() is synchronous and there are no other Embassy
        // tasks to starve. For multi-task applications, use
        // `embassy_time::Timer::after(Duration::from_millis(10)).await`.
        //
        // ~10 ms at 64 MHz (64_000_000 Hz / 100 Hz = 640_000 cycles).
        cortex_m::asm::delay(640_000);
    }
}
