//! Telepath RTT example for nRF52840-DK.
//!
//! Exposes five RPC commands over the Telepath wire:
//! - `ping`: sanity check, returns `0xDEADBEEF: u32`.
//! - `led_set(id: u8, on: bool)`: illuminate or extinguish one LED (id 1–4).
//! - `led_pattern(mask: u8)`: set all four LEDs in one round trip; bit 0 = LED1.
//! - `led_pattern_get()`: read back the current driven state of all four LEDs.
//! - `button_read()`: snapshot of all four button states; bit 0 = BTN1, pressed = 1.
//!
//! LED1–LED4 are fully under RPC control.  Liveness is indicated by periodic
//! `hb {n}` output on RTT channel 0 (visible in the RTT viewer).
//!
//! # Building
//!
//! ```
//! cd examples/nrf52840-ping
//! cargo build --release
//! ```
//!
//! # Flashing
//!
//! ```
//! cd examples/nrf52840-ping
//! cargo run --release
//! ```
#![no_std]
#![no_main]

mod rtt_transport;

use core::cell::RefCell;

use critical_section::Mutex;
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use panic_halt as _;
use rtt_target::{rprintln, rtt_init};
use telepath_server::{command, TelepathServer};

use rtt_transport::RttTransport;

// ---------------------------------------------------------------------------
// Shared GPIO state
//
// #[command] shims are plain free functions — they cannot capture locals from
// main.  We store the initialised GPIO handles in module-level statics guarded
// by a critical-section Mutex.  critical_section::Mutex<T> is Sync for any T;
// the single-core implementation (from cortex-m's critical-section-single-core
// feature) serialises access by disabling interrupts.
//
// Safety invariant for the `transmute` calls in `main`:
//   Output<'d> / Input<'d> store their AnyPin **by value** inside
//   PeripheralRef<'d, AnyPin>.  The lifetime parameter 'd is phantom
//   (PhantomData<&'d mut AnyPin>) — it has no runtime representation.
//   Transmuting Output<'_> → Output<'static> is therefore sound: the
//   nRF52840 peripheral tokens are 'static ZSTs, AnyPin carries no
//   borrowed data, and ownership is fully transferred into the static.
// ---------------------------------------------------------------------------

static LEDS: Mutex<RefCell<Option<[Output<'static>; 4]>>> = Mutex::new(RefCell::new(None));
static BTNS: Mutex<RefCell<Option<[Input<'static>; 4]>>> = Mutex::new(RefCell::new(None));

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Sanity check; returns `0xDEADBEEF`.
#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

/// Set one LED.  `id` must be 1–4; returns `false` and does nothing if out of range.
/// Active-low hardware: `on = true` drives the pin low to illuminate the LED.
#[command]
fn led_set(id: u8, on: bool) -> bool {
    if !(1..=4).contains(&id) {
        return false;
    }
    let idx = (id - 1) as usize;
    critical_section::with(|cs| {
        let mut guard = LEDS.borrow(cs).borrow_mut();
        if let Some(leds) = guard.as_mut() {
            if on {
                leds[idx].set_low();
            } else {
                leds[idx].set_high();
            }
            true
        } else {
            false
        }
    })
}

/// Set all four LEDs in one round trip.  Bit 0 = LED1, bit 3 = LED4.
/// Upper nibble is ignored.  Returns the applied mask (bits 0–3 only),
/// or `0` if the LED array has not been initialised.
#[command]
fn led_pattern(mask: u8) -> u8 {
    let m = mask & 0x0F;
    critical_section::with(|cs| {
        let mut guard = LEDS.borrow(cs).borrow_mut();
        if let Some(leds) = guard.as_mut() {
            for (i, led) in leds.iter_mut().enumerate() {
                // active-low: bit set → illuminate (set_low); bit clear → extinguish
                if (m >> i) & 1 == 1 {
                    led.set_low();
                } else {
                    led.set_high();
                }
            }
            m
        } else {
            0
        }
    })
}

/// Read back the current driven state of all four LEDs.
/// Bit 0 = LED1, bit 3 = LED4.  On (illuminated) = 1, Off = 0
/// (active-low hardware is inverted here).  Returns `0` if uninitialised.
#[command]
fn led_pattern_get() -> u8 {
    critical_section::with(|cs| {
        let guard = LEDS.borrow(cs).borrow();
        if let Some(leds) = guard.as_ref() {
            let mut state = 0u8;
            for (i, led) in leds.iter().enumerate() {
                if led.is_set_low() {
                    state |= 1 << i;
                }
            }
            state
        } else {
            0
        }
    })
}

/// Instantaneous snapshot of all four button states.
/// Bit 0 = BTN1, bit 3 = BTN4.  Pressed = 1 (active-low hardware, inverted here).
/// No wait semantics: the caller polls at its own rate.
#[command]
fn button_read() -> u8 {
    critical_section::with(|cs| {
        let guard = BTNS.borrow(cs).borrow();
        if let Some(btns) = guard.as_ref() {
            let mut state = 0u8;
            for (i, btn) in btns.iter().enumerate() {
                if btn.is_low() {
                    state |= 1 << i; // active-low: pin low → button pressed
                }
            }
            state
        } else {
            0
        }
    })
}

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
        // directly via ScanRegion::Exact — see tools/telepath-shell/src/rtt_transport.rs.
        section_cb: ".segger_rtt"
    };
    rtt_target::set_print_channel(channels.up.0);
    let rtt_transport = RttTransport::new(channels.up.1, channels.down.1);

    rprintln!("Telepath nRF52840-DK started");

    // LED1=P0.13, LED2=P0.14, LED3=P0.15, LED4=P0.16 (nRF52840-DK PCA10056, active-low).
    // Initial level High = all LEDs off at startup.
    let leds: [Output<'static>; 4] = unsafe {
        core::mem::transmute([
            Output::new(p.P0_13, Level::High, OutputDrive::Standard),
            Output::new(p.P0_14, Level::High, OutputDrive::Standard),
            Output::new(p.P0_15, Level::High, OutputDrive::Standard),
            Output::new(p.P0_16, Level::High, OutputDrive::Standard),
        ])
    };
    // BTN1=P0.11, BTN2=P0.12, BTN3=P0.24, BTN4=P0.25 (active-low, pull-up).
    let btns: [Input<'static>; 4] = unsafe {
        core::mem::transmute([
            Input::new(p.P0_11, Pull::Up),
            Input::new(p.P0_12, Pull::Up),
            Input::new(p.P0_24, Pull::Up),
            Input::new(p.P0_25, Pull::Up),
        ])
    };

    critical_section::with(|cs| {
        *LEDS.borrow(cs).borrow_mut() = Some(leds);
        *BTNS.borrow(cs).borrow_mut() = Some(btns);
    });

    let mut server =
        TelepathServer::<RttTransport, 512>::new(rtt_transport, telepath_server::commands());

    let mut tick: u32 = 0;
    let mut hb: u32 = 0;
    loop {
        // Process any pending RPC requests.
        server.poll();

        // Periodic heartbeat on RTT channel 0: visible in the RTT viewer as liveness indicator.
        // LED1–LED4 are now fully RPC-controlled and no longer used for blinking.
        if tick.is_multiple_of(100) {
            rprintln!("hb {}", hb);
            hb = hb.wrapping_add(1);
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
