//! Framing layer for the Telepath wire protocol.
//!
//! Both directions terminate frames with a `0x00` byte. The encoding
//! between frame bytes differs by direction:
//!
//! - Downstream (Host → Target): COBS
//! - Upstream   (Target → Host): rzCOBS
//!
//! [`FrameAccumulator`] is framing-agnostic — it discovers boundaries by
//! splitting on `0x00` and does not interpret the bytes between.
//!
//! # Framing-crate replacement policy
//!
//! Both COBS and rzCOBS are core wire infrastructure on the critical
//! path for every packet. The current implementations are thin wrappers
//! around the external `cobs` and `rzcobs` crates, but the stability
//! contract is the wrapper API exposed by this module:
//!
//! - [`cobs_encode`] / [`cobs_decode`] — downstream
//! - [`rzcobs_encode`] / [`rzcobs_decode`] — upstream
//!
//! Both algorithm pairs are interchangeable with an in-tree implementation
//! provided the wrapper signatures and [`crate::WireError`] mapping are
//! preserved. Replacement may be triggered (symmetrically for either) by
//! any of:
//!
//! 1. The upstream crate fails to build against a Rust edition / MSRV we
//!    need.
//! 2. A correctness or performance bug is identified and no upstream fix
//!    lands within 30 days.
//! 3. Optimizations specific to Telepath are wanted (e.g. exploiting the
//!    known [`crate::MAX_PAYLOAD_SIZE`] = 256 bound, or fusing the encode
//!    pass with postcard serialization).
//!
//! Reference materials for an in-tree rewrite:
//!
//! - COBS: Cheshire & Baker 1999; worst-case overhead `ceil(n / 254)`
//!   bytes, surfaced via `cobs::max_encoding_length`.
//! - rzCOBS: <https://github.com/Dirbaio/rzcobs#algorithm> (7-byte
//!   chunks; bitmap control byte; literal runs); worst-case overhead
//!   surfaced via [`max_rzcobs_encoding_length`].
//!
//! Unit tests in `mod tests` cover embedded zeros, long literal runs,
//! max-payload boundaries, and malformed input for **both** algorithms;
//! any in-tree replacement MUST pass them unchanged.

use crate::WireError;

/// Maximum COBS-encoded frame size including the `0x00` frame delimiter.
///
/// Sized to accommodate a fully-serialized `Request` or `Response` with a
/// maximum-length payload, plus framing overhead. For `MAX_PAYLOAD_SIZE = 256`,
/// a full serialized `Request` is at most ~264 bytes.
///
/// - COBS worst-case: `ceil(264 / 254)` ≈ 2 bytes overhead + 1 byte delimiter
///   → 267 bytes total.
/// - rzCOBS worst-case: `264 + ceil(264 / 7)` ≈ 38 bytes overhead + 1 byte
///   delimiter → 303 bytes total.
///
/// We round up to 512 to give headroom and match the RTT buffer size.
pub const MAX_FRAME_SIZE: usize = 512;

/// Worst-case rzCOBS-encoded length for a payload of `n` bytes, **excluding**
/// the trailing `0x00` frame delimiter.
///
/// Per Dirbaio's analysis: at most `ceil(n / 7)` control bytes are emitted,
/// plus `n` payload bytes, plus 1 for the final end-marker.
pub const fn max_rzcobs_encoding_length(n: usize) -> usize {
    n + n.div_ceil(7) + 1
}

/// COBS-encode `data` into `dst`, appending a `0x00` frame delimiter.
///
/// Returns the total bytes written to `dst` (encoded bytes + 1 for the
/// delimiter). `dst` must be at least `cobs::max_encoding_length(data.len()) + 1`
/// bytes long; [`MAX_FRAME_SIZE`] is always sufficient for any valid packet.
pub fn cobs_encode(data: &[u8], dst: &mut [u8]) -> Result<usize, WireError> {
    let min_len = cobs::max_encoding_length(data.len()) + 1;
    if dst.len() < min_len {
        return Err(WireError::PayloadTooLarge);
    }
    let n = cobs::encode(data, dst);
    dst[n] = 0x00;
    Ok(n + 1)
}

/// COBS-decode `src` (without the `0x00` delimiter) into `dst`.
///
/// Returns the number of decoded bytes written to `dst`.
pub fn cobs_decode(src: &[u8], dst: &mut [u8]) -> Result<usize, WireError> {
    cobs::decode(src, dst)
        .map(|report| report.frame_size())
        .map_err(|_| WireError::FramingError)
}

// ---------------------------------------------------------------------------
// rzCOBS helpers
// ---------------------------------------------------------------------------

/// A `&mut [u8]`-backed writer that implements the `rzcobs::Write` custom
/// trait. Fails with `()` when the slice is exhausted.
struct SliceWriter<'a> {
    dst: &'a mut [u8],
    pos: usize,
}

impl rzcobs::Write for SliceWriter<'_> {
    type Error = ();

    fn write(&mut self, byte: u8) -> Result<(), ()> {
        if self.pos >= self.dst.len() {
            return Err(());
        }
        self.dst[self.pos] = byte;
        self.pos += 1;
        Ok(())
    }
}

/// rzCOBS-encode `data` into `dst`, appending a `0x00` frame delimiter.
///
/// Returns the total bytes written to `dst` (encoded bytes + 1 for the
/// delimiter). `dst` must be at least
/// `max_rzcobs_encoding_length(data.len()) + 1` bytes long;
/// [`MAX_FRAME_SIZE`] is always sufficient for any valid packet.
pub fn rzcobs_encode(data: &[u8], dst: &mut [u8]) -> Result<usize, WireError> {
    let min_len = max_rzcobs_encoding_length(data.len()) + 1;
    if dst.len() < min_len {
        return Err(WireError::PayloadTooLarge);
    }
    // `Encoder::new` takes ownership of the writer; retrieve it via
    // `enc.writer()` after encoding is complete.
    let writer = SliceWriter { dst, pos: 0 };
    let mut enc = rzcobs::Encoder::new(writer);
    for &b in data {
        enc.write(b).map_err(|_| WireError::PayloadTooLarge)?;
    }
    enc.end().map_err(|_| WireError::PayloadTooLarge)?;
    // The pre-check guarantees n < dst.len(), so the delimiter write is safe.
    let n = enc.writer().pos;
    enc.writer().dst[n] = 0x00;
    Ok(n + 1)
}

/// rzCOBS-decode `src` (without the `0x00` delimiter) into `dst`.
///
/// Returns the number of decoded bytes written to `dst`.
///
/// # Trailing-zero padding
///
/// The rzCOBS algorithm works on 7-byte chunks. The last chunk is
/// zero-padded to 7 bytes, so the returned length may be up to 6 bytes
/// **larger** than the original data length. Callers that know the
/// exact original length should trim; callers passing the result to
/// `postcard::from_bytes` can ignore this because postcard silently
/// discards trailing bytes.
///
/// The `dst` buffer must be at least `MAX_FRAME_SIZE` bytes to
/// accommodate the worst-case padded output.
///
/// # In-tree implementation
///
/// The `rzcobs` crate v0.1.x does not provide a `no_std` decode
/// function, so this implementation follows the same algorithm directly.
/// The algorithm invariants are covered by the tests in `mod tests`.
pub fn rzcobs_decode(src: &[u8], dst: &mut [u8]) -> Result<usize, WireError> {
    let mut out_pos = 0usize;
    let mut it = src.iter().rev().copied();
    while let Some(x) = it.next() {
        match x {
            0x00 => return Err(WireError::FramingError),
            0x01..=0x7F => {
                for i in 0..7usize {
                    if out_pos >= dst.len() {
                        return Err(WireError::FramingError);
                    }
                    if x & (1 << (6 - i)) == 0 {
                        dst[out_pos] = it.next().ok_or(WireError::FramingError)?;
                    } else {
                        dst[out_pos] = 0x00;
                    }
                    out_pos += 1;
                }
            }
            0x80..=0xFE => {
                let n = usize::from(x & 0x7F) + 7;
                if out_pos >= dst.len() {
                    return Err(WireError::FramingError);
                }
                dst[out_pos] = 0x00;
                out_pos += 1;
                for _ in 0..n {
                    if out_pos >= dst.len() {
                        return Err(WireError::FramingError);
                    }
                    dst[out_pos] = it.next().ok_or(WireError::FramingError)?;
                    out_pos += 1;
                }
            }
            0xFF => {
                for _ in 0..134usize {
                    if out_pos >= dst.len() {
                        return Err(WireError::FramingError);
                    }
                    dst[out_pos] = it.next().ok_or(WireError::FramingError)?;
                    out_pos += 1;
                }
            }
        }
    }
    dst[..out_pos].reverse();
    Ok(out_pos)
}

// ---------------------------------------------------------------------------
// FrameAccumulator
// ---------------------------------------------------------------------------

/// Byte-by-byte frame accumulator for COBS-framed streams.
///
/// Feed raw bytes from the transport via [`Self::feed`]. When a `0x00`
/// delimiter is received, [`Self::frame`] returns the raw encoded frame
/// bytes ready for decoding.
///
/// `N` is the internal buffer capacity. Frames that exceed `N` bytes cause
/// the accumulator to discard the current frame and set an overflow flag;
/// [`Self::frame`] returns `None` until [`Self::reset`] is called.
pub struct FrameAccumulator<const N: usize> {
    buf: [u8; N],
    len: usize,
    overflow: bool,
}

impl<const N: usize> FrameAccumulator<N> {
    /// Create a new, empty accumulator.
    pub const fn new() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
            overflow: false,
        }
    }

    /// Feed one byte into the accumulator.
    ///
    /// Returns `true` when a complete frame has been received (i.e., a `0x00`
    /// delimiter was just observed). Call [`Self::frame`] to get the encoded
    /// bytes, then [`Self::reset`] before feeding more data.
    pub fn feed(&mut self, byte: u8) -> bool {
        if byte == 0x00 {
            // Frame delimiter — signal frame completion regardless of overflow.
            return true;
        }
        if self.len >= N {
            self.overflow = true;
            self.len = 0;
            return false;
        }
        self.buf[self.len] = byte;
        self.len += 1;
        false
    }

    /// Return the accumulated encoded frame bytes.
    ///
    /// Returns `None` if no complete frame is available (overflow or empty
    /// accumulator).
    pub fn frame(&self) -> Option<&[u8]> {
        if self.overflow || self.len == 0 {
            None
        } else {
            Some(&self.buf[..self.len])
        }
    }

    /// Reset the accumulator, discarding any partial frame.
    pub fn reset(&mut self) {
        self.len = 0;
        self.overflow = false;
    }
}

impl<const N: usize> Default for FrameAccumulator<N> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- COBS ---------------------------------------------------------------

    #[test]
    fn cobs_encode_decode_roundtrip() {
        let data = b"hello telepath";
        let mut encoded = [0u8; 64];
        let n = cobs_encode(data, &mut encoded).unwrap();
        assert_eq!(encoded[n - 1], 0x00);
        for &b in &encoded[..n - 1] {
            assert_ne!(b, 0x00);
        }
        let mut decoded = [0u8; 64];
        let m = cobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..m], data);
    }

    #[test]
    fn cobs_with_embedded_zeros() {
        let data = [0x00u8, 0x42, 0x00, 0xFF, 0x00];
        let mut encoded = [0u8; 32];
        let n = cobs_encode(&data, &mut encoded).unwrap();
        assert_eq!(encoded[n - 1], 0x00);
        let mut decoded = [0u8; 32];
        let m = cobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..m], &data);
    }

    #[test]
    fn cobs_long_run_no_zeros() {
        let data = [0x42u8; 300];
        let mut encoded = [0u8; 512];
        let n = cobs_encode(&data, &mut encoded).unwrap();
        assert_eq!(encoded[n - 1], 0x00);
        let mut decoded = [0u8; 512];
        let m = cobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..m], &data[..]);
    }

    #[test]
    fn cobs_max_payload_boundary() {
        let data = [0xABu8; crate::MAX_PAYLOAD_SIZE];
        let mut encoded = [0u8; MAX_FRAME_SIZE];
        let n = cobs_encode(&data, &mut encoded).unwrap();
        assert!(n <= MAX_FRAME_SIZE);
        let mut decoded = [0u8; MAX_FRAME_SIZE];
        let m = cobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..m], &data[..]);
    }

    #[test]
    fn cobs_encode_overflow_returns_error() {
        let data = b"hello";
        let mut tiny = [0u8; 1];
        assert!(matches!(
            cobs_encode(data, &mut tiny),
            Err(WireError::PayloadTooLarge)
        ));
    }

    #[test]
    fn cobs_decode_malformed_returns_error() {
        // A single 0x00 overhead byte (run-length of 0) is invalid in COBS.
        let bad = [0x00u8];
        let mut dst = [0u8; 16];
        assert!(matches!(
            cobs_decode(&bad, &mut dst),
            Err(WireError::FramingError)
        ));
    }

    // ---- FrameAccumulator ---------------------------------------------------

    #[test]
    fn accumulator_basic() {
        let mut acc: FrameAccumulator<64> = FrameAccumulator::new();
        let data = b"ping";
        let mut encoded = [0u8; 16];
        let n = cobs_encode(data, &mut encoded).unwrap();
        let mut complete = false;
        for &b in &encoded[..n] {
            complete = acc.feed(b);
        }
        assert!(complete);
        let frame = acc.frame().unwrap();
        let mut decoded = [0u8; 16];
        let m = cobs_decode(frame, &mut decoded).unwrap();
        assert_eq!(&decoded[..m], data);
    }

    #[test]
    fn accumulator_reset_allows_second_frame() {
        let mut acc: FrameAccumulator<64> = FrameAccumulator::new();
        let data1 = b"first";
        let data2 = b"second";
        let mut enc = [0u8; 32];

        let n = cobs_encode(data1, &mut enc).unwrap();
        for &b in &enc[..n] {
            acc.feed(b);
        }
        acc.reset();

        let n = cobs_encode(data2, &mut enc).unwrap();
        let mut complete = false;
        for &b in &enc[..n] {
            complete = acc.feed(b);
        }
        assert!(complete);
        let frame = acc.frame().unwrap();
        let mut decoded = [0u8; 32];
        let m = cobs_decode(frame, &mut decoded).unwrap();
        assert_eq!(&decoded[..m], data2);
    }

    #[test]
    fn accumulator_overflow_returns_none() {
        let mut acc: FrameAccumulator<4> = FrameAccumulator::new();
        for _ in 0..5 {
            acc.feed(0x42);
        }
        acc.feed(0x00);
        assert!(acc.frame().is_none());
    }

    #[test]
    fn max_frame_size_covers_max_payload() {
        assert!(MAX_FRAME_SIZE >= crate::MAX_PAYLOAD_SIZE + 4);
    }

    // ---- rzCOBS -------------------------------------------------------------

    #[test]
    fn rzcobs_encode_decode_roundtrip() {
        // "hello telepath" is 14 bytes = exactly 2 full 7-byte chunks,
        // so there is no trailing-zero padding.
        let data = b"hello telepath";
        let mut encoded = [0u8; 64];
        let n = rzcobs_encode(data, &mut encoded).unwrap();
        assert_eq!(encoded[n - 1], 0x00);
        let mut decoded = [0u8; 64];
        let m = rzcobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..data.len()], data.as_slice());
        assert!(decoded[data.len()..m].iter().all(|&b| b == 0x00));
    }

    #[test]
    fn rzcobs_with_embedded_zeros() {
        // Decoded output is padded to 7-byte boundary; last bytes are zeros.
        let data = [0x00u8, 0x42, 0x00, 0xFF, 0x00];
        let mut encoded = [0u8; 32];
        let n = rzcobs_encode(&data, &mut encoded).unwrap();
        assert_eq!(encoded[n - 1], 0x00);
        let mut decoded = [0u8; 32];
        let m = rzcobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..data.len()], &data);
        assert!(decoded[data.len()..m].iter().all(|&b| b == 0x00));
    }

    #[test]
    fn rzcobs_long_run_no_zeros() {
        // 200 bytes → ceil(200/7) = 29 chunks → 203 decoded bytes.
        let data = [0x42u8; 200];
        let mut encoded = [0u8; 512];
        let n = rzcobs_encode(&data, &mut encoded).unwrap();
        assert_eq!(encoded[n - 1], 0x00);
        let mut decoded = [0u8; 512];
        let m = rzcobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..data.len()], &data[..]);
        assert!(decoded[data.len()..m].iter().all(|&b| b == 0x00));
    }

    #[test]
    fn rzcobs_max_payload_boundary() {
        // 256 bytes → ceil(256/7) = 37 chunks → 259 decoded bytes.
        let data = [0xABu8; crate::MAX_PAYLOAD_SIZE];
        let mut encoded = [0u8; MAX_FRAME_SIZE];
        let n = rzcobs_encode(&data, &mut encoded).unwrap();
        assert!(n <= MAX_FRAME_SIZE);
        let mut decoded = [0u8; MAX_FRAME_SIZE];
        let m = rzcobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..data.len()], &data[..]);
        assert!(decoded[data.len()..m].iter().all(|&b| b == 0x00));
    }

    #[test]
    fn rzcobs_encode_overflow_returns_error() {
        let data = b"hello";
        let mut tiny = [0u8; 1];
        assert!(matches!(
            rzcobs_encode(data, &mut tiny),
            Err(WireError::PayloadTooLarge)
        ));
    }

    #[test]
    fn rzcobs_decode_malformed_returns_error() {
        // A single 0x01 byte is an invalid rzCOBS frame (incomplete chunk).
        let bad = [0x01u8];
        let mut dst = [0u8; 16];
        assert!(matches!(
            rzcobs_decode(&bad, &mut dst),
            Err(WireError::FramingError)
        ));
    }

    #[test]
    fn rzcobs_max_frame_size_covers_max_payload() {
        assert!(MAX_FRAME_SIZE >= max_rzcobs_encoding_length(crate::MAX_PAYLOAD_SIZE) + 1);
    }
}
