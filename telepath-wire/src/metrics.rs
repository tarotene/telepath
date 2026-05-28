use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

/// Per-direction framing metrics accumulated since the last snapshot reset.
///
/// Returned by CmdID [`super::CMD_ID_METRICS`] (`0xFFFE`). The target
/// atomically resets all counters when it replies.
///
/// Cycle counts are DWT cycle counter deltas (Cortex-M4F @ firmware clock
/// frequency). Convert to wall-clock duration by dividing by the CPU
/// frequency in Hz.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, Schema)]
pub struct MetricsSnapshot {
    /// Total DWT cycles spent in `rzcobs_encode` (upstream, target→host).
    pub encode_cycles: u64,
    /// Total DWT cycles spent in `rzcobs_decode` (downstream, host→target).
    pub decode_cycles: u64,
    /// Total payload bytes input to `rzcobs_encode` (pre-framing size).
    pub encoded_bytes: u32,
    /// Total payload bytes output by `rzcobs_decode` (post-unframe size).
    pub decoded_bytes: u32,
    /// Number of complete request/response round trips counted.
    pub sample_count: u32,
}
