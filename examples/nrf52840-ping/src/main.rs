//! Telepath RTT example for nRF52840-DK.
//!
//! Exposes twelve RPC commands over the Telepath wire:
//! - `ping`: sanity check, returns `0xDEADBEEF: u32`.
//! - `add(a: i32, b: i32)`: returns `a + b`.
//! - `crc32(payload)`: CRC-32/ISO-HDLC over the payload bytes.
//! - `echo(payload)`: returns the payload unchanged.
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

use embassy_executor::Spawner;
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use nrf_pac as pac;
use panic_halt as _;
use rtt_target::{rprintln, rtt_init};
use heapless::Vec as HVec;
use telepath_server::{command, TelepathServer};

use rtt_transport::RttTransport;

// ---------------------------------------------------------------------------
// Resource newtypes
//
// Each GPIO pin is wrapped in a dedicated newtype so the `#[resource]`
// injection system can distinguish them by `TypeId`. The `new()` constructors
// encapsulate the `transmute` that erases the HAL lifetime parameter.
//
// Safety invariant for the `transmute` inside `new()`:
//   Output<'d> / Input<'d> store their AnyPin **by value** inside
//   PeripheralRef<'d, AnyPin>.  The lifetime parameter 'd is phantom
//   (PhantomData<&'d mut AnyPin>) — it has no runtime representation.
//   Transmuting Output<'_> → Output<'static> is therefore sound: the
//   nRF52840 peripheral tokens are 'static ZSTs, AnyPin carries no
//   borrowed data, and ownership is fully transferred into the newtype.
// ---------------------------------------------------------------------------

pub struct Led1(pub Output<'static>);
pub struct Led2(pub Output<'static>);
pub struct Led3(pub Output<'static>);
pub struct Led4(pub Output<'static>);
pub struct Btn1(pub Input<'static>);
pub struct Btn2(pub Input<'static>);
pub struct Btn3(pub Input<'static>);
pub struct Btn4(pub Input<'static>);

macro_rules! impl_led_newtype {
    ($name:ident) => {
        impl $name {
            pub fn new(pin: Output<'_>) -> Self {
                // SAFETY: see "Safety invariant" comment above.
                Self(unsafe { core::mem::transmute::<Output<'_>, Output<'static>>(pin) })
            }
        }
    };
}

macro_rules! impl_btn_newtype {
    ($name:ident) => {
        impl $name {
            pub fn new(pin: Input<'_>) -> Self {
                // SAFETY: see "Safety invariant" comment above.
                Self(unsafe { core::mem::transmute::<Input<'_>, Input<'static>>(pin) })
            }
        }
    };
}

impl_led_newtype!(Led1);
impl_led_newtype!(Led2);
impl_led_newtype!(Led3);
impl_led_newtype!(Led4);
impl_btn_newtype!(Btn1);
impl_btn_newtype!(Btn2);
impl_btn_newtype!(Btn3);
impl_btn_newtype!(Btn4);

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Sanity check; returns `0xDEADBEEF`.
#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

/// Returns `a + b`.
#[command]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn crc32_iso_hdlc(data: &[u8]) -> u32 {
    // CRC-32/ISO-HDLC: poly=0x04C11DB7, refin=true, refout=true,
    // init=0xFFFF_FFFF, xorout=0xFFFF_FFFF.
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// CRC-32/ISO-HDLC over the provided payload.
#[command]
fn crc32(payload: HVec<u8, 128>) -> u32 {
    crc32_iso_hdlc(&payload)
}

/// Returns the payload unchanged.
#[command]
fn echo(payload: HVec<u8, 128>) -> HVec<u8, 128> {
    payload
}

/// Set one LED.  `id` must be 1–4; returns `false` and does nothing if out of range.
/// Active-low hardware: `on = true` drives the pin low to illuminate the LED.
#[command]
fn led_set(
    #[resource] led1: &mut Led1,
    #[resource] led2: &mut Led2,
    #[resource] led3: &mut Led3,
    #[resource] led4: &mut Led4,
    id: u8,
    on: bool,
) -> bool {
    let led = match id {
        1 => &mut led1.0,
        2 => &mut led2.0,
        3 => &mut led3.0,
        4 => &mut led4.0,
        _ => return false,
    };
    if on {
        led.set_low();
    } else {
        led.set_high();
    }
    true
}

/// Set all four LEDs in one round trip.  Bit 0 = LED1, bit 3 = LED4.
/// Upper nibble is ignored.  Returns the applied mask (bits 0–3 only).
#[command]
fn led_pattern(
    #[resource] led1: &mut Led1,
    #[resource] led2: &mut Led2,
    #[resource] led3: &mut Led3,
    #[resource] led4: &mut Led4,
    mask: u8,
) -> u8 {
    let m = mask & 0x0F;
    let mut leds: [&mut Output<'static>; 4] = [&mut led1.0, &mut led2.0, &mut led3.0, &mut led4.0];
    for (i, led) in leds.iter_mut().enumerate() {
        if (m >> i) & 1 == 1 {
            led.set_low();
        } else {
            led.set_high();
        }
    }
    m
}

/// Read back the current driven state of all four LEDs.
/// Bit 0 = LED1, bit 3 = LED4.  On (illuminated) = 1, Off = 0
/// (active-low hardware is inverted here).
#[command]
fn led_pattern_get(
    #[resource] led1: &Led1,
    #[resource] led2: &Led2,
    #[resource] led3: &Led3,
    #[resource] led4: &Led4,
) -> u8 {
    let leds: [&Output<'static>; 4] = [&led1.0, &led2.0, &led3.0, &led4.0];
    let mut state = 0u8;
    for (i, led) in leds.iter().enumerate() {
        if led.is_set_low() {
            state |= 1 << i;
        }
    }
    state
}

/// Instantaneous snapshot of all four button states.
/// Bit 0 = BTN1, bit 3 = BTN4.  Pressed = 1 (active-low hardware, inverted here).
/// No wait semantics: the caller polls at its own rate.
#[command]
fn button_read(
    #[resource] btn1: &Btn1,
    #[resource] btn2: &Btn2,
    #[resource] btn3: &Btn3,
    #[resource] btn4: &Btn4,
) -> u8 {
    let btns: [&Input<'static>; 4] = [&btn1.0, &btn2.0, &btn3.0, &btn4.0];
    let mut state = 0u8;
    for (i, btn) in btns.iter().enumerate() {
        if btn.is_low() {
            state |= 1 << i;
        }
    }
    state
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
    // BTN1=P0.11, BTN2=P0.12, BTN3=P0.24, BTN4=P0.25 (active-low, pull-up).
    let mut server = TelepathServer::<RttTransport, 512>::new(
        rtt_transport,
        telepath_server::commands(),
    )
    .resource(Led1::new(Output::new(p.P0_13, Level::High, OutputDrive::Standard)))
    .resource(Led2::new(Output::new(p.P0_14, Level::High, OutputDrive::Standard)))
    .resource(Led3::new(Output::new(p.P0_15, Level::High, OutputDrive::Standard)))
    .resource(Led4::new(Output::new(p.P0_16, Level::High, OutputDrive::Standard)))
    .resource(Btn1::new(Input::new(p.P0_11, Pull::Up)))
    .resource(Btn2::new(Input::new(p.P0_12, Pull::Up)))
    .resource(Btn3::new(Input::new(p.P0_24, Pull::Up)))
    .resource(Btn4::new(Input::new(p.P0_25, Pull::Up)));

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
