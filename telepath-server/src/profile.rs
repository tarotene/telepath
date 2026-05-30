//! DWT-based framing instrumentation (enabled by the `profile` feature).
//!
//! Accumulates cycle counts and byte counts for each COBS encode/decode
//! operation in `process_frame`. The counters are stored in module-level
//! statics so they survive across multiple poll() calls.
//!
//! Call [`init_dwt`] once at startup (done automatically by
//! [`TelepathServer::new`] when `profile` is enabled) to enable the
//! Cortex-M DWT cycle counter.
//!
//! Retrieve and atomically reset all counters by calling
//! [`snapshot_and_reset`], or equivalently by sending CmdID `0xFFFE`
//! (`CMD_ID_METRICS`) from the host.

// On targets with native 64-bit atomics (e.g. x86_64 host), use core directly.
// On 32-bit embedded targets (e.g. Cortex-M4), fall back to portable-atomic
// which requires the caller to enable "unsafe-assume-single-core" + "fallback"
// features in their own Cargo.toml (see examples/nrf52840-ping).
#[cfg(target_has_atomic = "64")]
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
#[cfg(not(target_has_atomic = "64"))]
use portable_atomic::{AtomicU32, AtomicU64, Ordering};

use telepath_wire::MetricsSnapshot;

pub(crate) static ENCODE_CYCLES: AtomicU64 = AtomicU64::new(0);
pub(crate) static DECODE_CYCLES: AtomicU64 = AtomicU64::new(0);
pub(crate) static ENCODED_BYTES: AtomicU32 = AtomicU32::new(0);
pub(crate) static DECODED_BYTES: AtomicU32 = AtomicU32::new(0);
pub(crate) static SAMPLE_COUNT: AtomicU32 = AtomicU32::new(0);

/// Enable the Cortex-M DWT cycle counter.
///
/// On ARM targets: sets `DEMCR.TRCENA`, enables the cycle counter, and resets
/// `CYCCNT` to 0. Uses `Peripherals::steal` so user code does not need to give
/// up the singleton. Idempotent — safe to call multiple times.
///
/// On non-ARM targets (e.g. x86_64 host-pty-server): no-op. The atomic counters
/// still accumulate but `cycles_now()` always returns 0, so cycle fields in the
/// snapshot will be 0. Host-side `Instant` timing in `telepath-client` remains
/// meaningful for bench-pty.
///
/// # Safety
///
/// On ARM: uses `cortex_m::peripheral::Peripherals::steal()`. Only sound on
/// single-core Cortex-M targets with `profile` intentionally enabled.
pub fn init_dwt() {
    #[cfg(target_arch = "arm")]
    unsafe {
        let mut cp = cortex_m::peripheral::Peripherals::steal();
        cp.DCB.enable_trace();
        cortex_m::peripheral::DWT::unlock();
        cp.DWT.enable_cycle_counter();
        cp.DWT.cyccnt.write(0);
    }
}

/// Read the current DWT cycle counter value.
/// Returns 0 on non-ARM targets (e.g. x86_64 host-pty-server).
#[inline(always)]
pub fn cycles_now() -> u32 {
    #[cfg(target_arch = "arm")]
    {
        cortex_m::peripheral::DWT::cycle_count()
    }
    #[cfg(not(target_arch = "arm"))]
    {
        0
    }
}

/// Return the current metrics snapshot and atomically reset all counters.
pub fn snapshot_and_reset() -> MetricsSnapshot {
    MetricsSnapshot {
        encode_cycles: ENCODE_CYCLES.swap(0, Ordering::Relaxed),
        decode_cycles: DECODE_CYCLES.swap(0, Ordering::Relaxed),
        encoded_bytes: ENCODED_BYTES.swap(0, Ordering::Relaxed),
        decoded_bytes: DECODED_BYTES.swap(0, Ordering::Relaxed),
        sample_count: SAMPLE_COUNT.swap(0, Ordering::Relaxed),
    }
}

// ---------------------------------------------------------------------------
// CmdID 0xFFFE registration
//
// We cannot use the `#[command]` proc-macro inside `telepath-server` itself
// because the macro emits `::telepath_server::...` paths that do not resolve
// when compiling the crate itself. Instead we build the CommandMetadata and
// the linkme slice entry by hand — equivalent to what the macro would emit.
// TODO(#76): after rzCOBS upstream lands, ENCODE_CYCLES tracks rzcobs_encode
// instead of cobs_encode. The metric semantics remain valid after the swap.
// ---------------------------------------------------------------------------

fn get_metrics_shim(
    input: &[u8],
    output: &mut [u8],
    _resources: &crate::ResourceRegistry,
) -> Result<crate::DispatchOutcome, crate::DispatchError> {
    if !input.is_empty() {
        return Err(crate::DispatchError::DeserializeError);
    }
    let snap = snapshot_and_reset();
    postcard::to_slice(&snap, output)
        .map(|s| crate::DispatchOutcome::Ok(s.len()))
        .map_err(|_| crate::DispatchError::SerializeError)
}

fn get_metrics_args_schema(out: &mut [u8]) -> Result<usize, ()> {
    postcard::to_slice(<() as crate::__postcard_schema::Schema>::SCHEMA, out)
        .map(|s| s.len())
        .map_err(|_| ())
}

fn get_metrics_ret_schema(out: &mut [u8]) -> Result<usize, ()> {
    postcard::to_slice(
        <MetricsSnapshot as crate::__postcard_schema::Schema>::SCHEMA,
        out,
    )
    .map(|s| s.len())
    .map_err(|_| ())
}

pub const GET_METRICS_CMD: crate::CommandMetadata = crate::CommandMetadata {
    name: "get_metrics",
    id: telepath_wire::CMD_ID_METRICS,
    invoke: get_metrics_shim,
    args_schema: get_metrics_args_schema,
    ret_schema: get_metrics_ret_schema,
    arg_names: "",
};

#[allow(non_upper_case_globals, non_snake_case)]
#[crate::__linkme::distributed_slice(crate::TELEPATH_COMMANDS)]
#[linkme(crate = crate::__linkme)]
static __TELEPATH_REG_GET_METRICS: crate::CommandMetadata = GET_METRICS_CMD;
