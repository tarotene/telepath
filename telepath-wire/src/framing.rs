//! COBS framing layer for the Telepath wire protocol.
//!
//! Both downstream (Host→Target) and upstream (Target→Host) use COBS framing
//! in this MVP implementation. A `0x00` byte serves as the frame delimiter.
//!
//! # Future work
//!
//! The spec calls for rzCOBS on the upstream path. Switching upstream to rzCOBS
//! requires replacing `cobs_encode` with an rzCOBS encoder in the firmware
//! `poll()` loop. The `FrameAccumulator` is framing-agnostic and does not
//! need to change.
//!
// TODO(upstream): replace firmware→host direction with rzCOBS once the host
// has an rzCOBS decoder in place.

use crate::WireError;

/// Maximum COBS-encoded frame size including the `0x00` frame delimiter.
///
/// Sized to accommodate a fully-serialized `Request` or `Response` with a
/// maximum-length payload, plus COBS overhead. For `MAX_PAYLOAD_SIZE = 256`,
/// a full serialized `Request` is at most ~264 bytes; COBS adds at most 2
/// bytes overhead; the delimiter adds 1 byte → 267 bytes. We round up to 512
/// to give headroom and match the RTT buffer size.
pub const MAX_FRAME_SIZE: usize = 512;

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
    cobs::decode(src, dst).map_err(|_| WireError::FramingError)
}

// ---------------------------------------------------------------------------
// FrameAccumulator
// ---------------------------------------------------------------------------

/// Byte-by-byte frame accumulator for COBS-framed streams.
///
/// Feed raw bytes from the transport via [`Self::feed`]. When a `0x00`
/// delimiter is received, [`Self::frame`] returns the raw COBS-encoded frame
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

    /// Return the accumulated COBS-encoded frame bytes.
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

    #[test]
    fn encode_decode_roundtrip() {
        let data = b"hello telepath";
        let mut encoded = [0u8; 64];
        let n = cobs_encode(data, &mut encoded).unwrap();
        // Last byte must be the 0x00 delimiter.
        assert_eq!(encoded[n - 1], 0x00);
        // No 0x00 bytes in the encoded payload.
        for &b in &encoded[..n - 1] {
            assert_ne!(b, 0x00);
        }
        let mut decoded = [0u8; 64];
        let m = cobs_decode(&encoded[..n - 1], &mut decoded).unwrap();
        assert_eq!(&decoded[..m], data);
    }

    #[test]
    fn accumulator_basic() {
        let mut acc: FrameAccumulator<64> = FrameAccumulator::new();
        let data = b"ping";
        let mut encoded = [0u8; 16];
        let n = cobs_encode(data, &mut encoded).unwrap();
        // Feed all bytes including the 0x00 delimiter.
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
        // Feed 5 non-zero bytes → overflow.
        for _ in 0..5 {
            acc.feed(0x42);
        }
        acc.feed(0x00); // delimiter
        assert!(acc.frame().is_none());
    }

    #[test]
    fn max_frame_size_covers_max_payload() {
        assert!(MAX_FRAME_SIZE >= crate::MAX_PAYLOAD_SIZE + 4);
    }
}
