//! Telepath RTT example for nRF52840-DK.
//!
//! Exposes nine RPC commands over the Telepath wire:
//! - `ping`: sanity check, returns `0xDEADBEEF: u32`.
//! - `led_set(id: u8, on: bool)`: illuminate or extinguish one LED (id 1–4).
//! - `led_pattern(mask: u8)`: set all four LEDs in one round trip; bit 0 = LED1.
//! - `led_pattern_get()`: read back the current driven state of all four LEDs.
//! - `button_read()`: snapshot of all four button states; bit 0 = BTN1, pressed = 1.
//! - `ficr_uid()`: unique 64-bit chip ID from FICR.DEVICEID[0..1].
//! - `temp_read()`: die temperature in 0.25 °C units from the on-chip TEMP peripheral.
//! - `rng_u32()`: true random u32 from the hardware RNG (bias-corrected).
//! - `saadc_vdd_mv()`: supply voltage in millivolts via the SAADC VDD channel.
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
use nrf_pac as pac;
use panic_halt as _;
use rtt_target::{rprintln, rtt_init};
use telepath_server::{command, TelepathServer};

use rtt_transport::RttTransport;

// ---------------------------------------------------------------------------
// Per-pin GPIO statics
//
// #[command] shims are plain free functions — they cannot capture locals from
// main.  GPIO handles are stored as individual module-level statics, one per
// pin.  Each static wraps its handle in critical_section::Mutex<RefCell<Option<T>>>
// to satisfy the Sync bound (critical-section-single-core disables interrupts).
//
// Pins are stored individually rather than as a single bundled array because
// each GPIO pin is an independent hardware resource.  The nRF52840 uses
// OUTSET/OUTCLR registers for atomic per-bit writes, so there is no contention
// between different pins and no reason to serialise access to all four under
// a single lock.
//
// Safety invariant for the `transmute` calls in `main`:
//   Output<'d> / Input<'d> store their AnyPin **by value** inside
//   PeripheralRef<'d, AnyPin>.  The lifetime parameter 'd is phantom
//   (PhantomData<&'d mut AnyPin>) — it has no runtime representation.
//   Transmuting Output<'_> → Output<'static> is therefore sound: the
//   nRF52840 peripheral tokens are 'static ZSTs, AnyPin carries no
//   borrowed data, and ownership is fully transferred into the static.
// ---------------------------------------------------------------------------

static LED1: Mutex<RefCell<Option<Output<'static>>>> = Mutex::new(RefCell::new(None));
static LED2: Mutex<RefCell<Option<Output<'static>>>> = Mutex::new(RefCell::new(None));
static LED3: Mutex<RefCell<Option<Output<'static>>>> = Mutex::new(RefCell::new(None));
static LED4: Mutex<RefCell<Option<Output<'static>>>> = Mutex::new(RefCell::new(None));

static BTN1: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));
static BTN2: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));
static BTN3: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));
static BTN4: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Sanity check; returns `0xDEADBEEF`.
#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

fn led_mux(id: u8) -> Option<&'static Mutex<RefCell<Option<Output<'static>>>>> {
    Some(match id {
        1 => &LED1,
        2 => &LED2,
        3 => &LED3,
        4 => &LED4,
        _ => return None,
    })
}

/// Set one LED.  `id` must be 1–4; returns `false` and does nothing if out of range.
/// Active-low hardware: `on = true` drives the pin low to illuminate the LED.
#[command]
fn led_set(id: u8, on: bool) -> bool {
    let Some(mux) = led_mux(id) else {
        return false;
    };
    critical_section::with(|cs| {
        let mut guard = mux.borrow(cs).borrow_mut();
        if let Some(led) = guard.as_mut() {
            if on {
                led.set_low();
            } else {
                led.set_high();
            }
            true
        } else {
            false
        }
    })
}

/// Set all four LEDs in one round trip.  Bit 0 = LED1, bit 3 = LED4.
/// Upper nibble is ignored.  Returns the applied mask (bits 0–3 only),
/// or `0` if any LED has not been initialised.
#[command]
fn led_pattern(mask: u8) -> u8 {
    let m = mask & 0x0F;
    let leds = [&LED1, &LED2, &LED3, &LED4];
    critical_section::with(|cs| {
        for (i, mux) in leds.iter().enumerate() {
            let mut guard = mux.borrow(cs).borrow_mut();
            let Some(led) = guard.as_mut() else {
                return 0;
            };
            // active-low: bit set → illuminate (set_low); bit clear → extinguish
            if (m >> i) & 1 == 1 {
                led.set_low();
            } else {
                led.set_high();
            }
        }
        m
    })
}

/// Read back the current driven state of all four LEDs.
/// Bit 0 = LED1, bit 3 = LED4.  On (illuminated) = 1, Off = 0
/// (active-low hardware is inverted here).  Returns `0` if any LED is uninitialised.
#[command]
fn led_pattern_get() -> u8 {
    let leds = [&LED1, &LED2, &LED3, &LED4];
    critical_section::with(|cs| {
        let mut state = 0u8;
        for (i, mux) in leds.iter().enumerate() {
            let guard = mux.borrow(cs).borrow();
            let Some(led) = guard.as_ref() else {
                return 0;
            };
            if led.is_set_low() {
                state |= 1 << i;
            }
        }
        state
    })
}

/// Instantaneous snapshot of all four button states.
/// Bit 0 = BTN1, bit 3 = BTN4.  Pressed = 1 (active-low hardware, inverted here).
/// No wait semantics: the caller polls at its own rate.
#[command]
fn button_read() -> u8 {
    let btns = [&BTN1, &BTN2, &BTN3, &BTN4];
    critical_section::with(|cs| {
        let mut state = 0u8;
        for (i, mux) in btns.iter().enumerate() {
            let guard = mux.borrow(cs).borrow();
            let Some(btn) = guard.as_ref() else {
                return 0;
            };
            if btn.is_low() {
                state |= 1 << i; // active-low: pin low → button pressed
            }
        }
        state
    })
}

/// Unique 64-bit chip ID from FICR.DEVICEID[0..1].
/// Pure MMIO read — no peripheral initialization required.
/// The value is factory-programmed, stable across reboots, and unique per die.
/// Example (one particular nRF52840-DK): `(0x893CE2C0, 0xA94DF961)`.
/// Your board will return different values; treat as an opaque identifier.
#[command]
fn ficr_uid() -> (u32, u32) {
    (pac::FICR.deviceid(0).read(), pac::FICR.deviceid(1).read())
}

/// Die temperature in 0.25 °C units.
/// Returns a signed integer: divide by 4 to get °C, or multiply by 250 for m°C.
/// Example: 100 → 25.00 °C; 111 → 27.75 °C.
/// Operating range: −160…340 (nRF52840 specification: −40 °C to 85 °C).
///
/// Drives `pac::TEMP` directly (busy-poll) to satisfy the synchronous `#[command]` contract;
/// the `embassy_nrf::temp::Temp` driver is async-only.
#[command]
fn temp_read() -> i16 {
    let t = pac::TEMP;
    t.events_datardy().write_value(0); // clear stale event before starting
    t.tasks_start().write_value(1);
    while t.events_datardy().read() == 0 {}
    let raw = t.temp().read() as i32;
    t.events_datardy().write_value(0);
    t.tasks_stop().write_value(1);
    raw as i16
}

/// True random u32 from the hardware RNG with bias correction enabled.
/// Each call generates 4 fresh random bytes; values should differ across calls.
///
/// Drives `pac::RNG` directly (busy-poll per byte) to satisfy the synchronous
/// `#[command]` contract. Bias correction adds ~167 ns per bit on average but
/// eliminates LFSR output bias.
#[command]
fn rng_u32() -> u32 {
    let r = pac::RNG;
    r.config().write(|w| w.set_dercen(true));
    r.events_valrdy().write_value(0); // clear stale event before starting
    r.tasks_start().write_value(1);
    let mut bytes = [0u8; 4];
    for byte in bytes.iter_mut() {
        while r.events_valrdy().read() == 0 {}
        *byte = r.value().read().value(); // read VALUE before clearing VALRDY
        r.events_valrdy().write_value(0);
    }
    r.tasks_stop().write_value(1);
    u32::from_le_bytes(bytes)
}

/// Supply voltage in millivolts via the SAADC VDD internal channel.
/// Typical value: ~3300 mV under USB power, ~3000 mV from a coin cell.
///
/// Drives `pac::SAADC` directly to satisfy the synchronous `#[command]` contract;
/// the `embassy_nrf::saadc::Saadc` driver is async-only.
/// Configuration: 10-bit, GAIN=1/6, VREF=0.6 V → full-scale input = 3.6 V.
/// Conversion: VDD_mV = raw_count × 3600 / 1024.
#[command]
fn saadc_vdd_mv() -> u16 {
    use pac::saadc::vals;
    let r = pac::SAADC;
    r.resolution().write(|w| w.set_val(vals::Val::_10BIT));
    r.oversample().write(|w| w.set_oversample(vals::Oversample::BYPASS));
    r.ch(0).pselp().write(|w| w.set_pselp(vals::Psel::VDD));
    r.ch(0).config().write(|w| {
        w.set_refsel(vals::Refsel::INTERNAL);
        w.set_gain(vals::Gain::GAIN1_6);
        w.set_tacq(vals::Tacq::_10US);
        w.set_mode(vals::ConfigMode::SE);
    });
    // DMA result buffer: one i16 sample on the stack.
    // `read_volatile` after the END event prevents the optimizer from eliding
    // the DMA-written value (the write is invisible to the compiler).
    let mut buf: i16 = 0;
    r.result()
        .ptr()
        .write_value(core::ptr::addr_of_mut!(buf) as u32);
    r.result().maxcnt().write(|w| w.set_maxcnt(1));
    r.enable().write(|w| w.set_enable(true));
    r.events_started().write_value(0); // clear stale events before starting
    r.events_end().write_value(0);
    r.tasks_start().write_value(1);
    // Wait for SAADC to be ready before issuing SAMPLE (nRF52840 PS §6.23.4).
    while r.events_started().read() == 0 {}
    r.events_started().write_value(0);
    r.tasks_sample().write_value(1);
    while r.events_end().read() == 0 {}
    r.events_end().write_value(0);
    r.tasks_stop().write_value(1);
    r.enable().write(|w| w.set_enable(false));
    let raw = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(buf)) };
    // VDD_mV = raw × 3600 / 1024  (GAIN=1/6, VREF=0.6 V, 10-bit → full-scale = 2^10)
    ((raw as i32) * 3600 / 1024).max(0) as u16
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
    let led1: Output<'static> =
        unsafe { core::mem::transmute(Output::new(p.P0_13, Level::High, OutputDrive::Standard)) };
    let led2: Output<'static> =
        unsafe { core::mem::transmute(Output::new(p.P0_14, Level::High, OutputDrive::Standard)) };
    let led3: Output<'static> =
        unsafe { core::mem::transmute(Output::new(p.P0_15, Level::High, OutputDrive::Standard)) };
    let led4: Output<'static> =
        unsafe { core::mem::transmute(Output::new(p.P0_16, Level::High, OutputDrive::Standard)) };

    // BTN1=P0.11, BTN2=P0.12, BTN3=P0.24, BTN4=P0.25 (active-low, pull-up).
    let btn1: Input<'static> = unsafe { core::mem::transmute(Input::new(p.P0_11, Pull::Up)) };
    let btn2: Input<'static> = unsafe { core::mem::transmute(Input::new(p.P0_12, Pull::Up)) };
    let btn3: Input<'static> = unsafe { core::mem::transmute(Input::new(p.P0_24, Pull::Up)) };
    let btn4: Input<'static> = unsafe { core::mem::transmute(Input::new(p.P0_25, Pull::Up)) };

    critical_section::with(|cs| {
        *LED1.borrow(cs).borrow_mut() = Some(led1);
        *LED2.borrow(cs).borrow_mut() = Some(led2);
        *LED3.borrow(cs).borrow_mut() = Some(led3);
        *LED4.borrow(cs).borrow_mut() = Some(led4);
        *BTN1.borrow(cs).borrow_mut() = Some(btn1);
        *BTN2.borrow(cs).borrow_mut() = Some(btn2);
        *BTN3.borrow(cs).borrow_mut() = Some(btn3);
        *BTN4.borrow(cs).borrow_mut() = Some(btn4);
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
