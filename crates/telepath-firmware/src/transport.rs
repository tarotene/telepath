//! Non-blocking byte-stream transport trait.
//!
//! Any byte-stream I/O source/sink implements [`Transport`]. For nRF52840-DK,
//! see `rtt_transport::RttTransport` in the example crate.

/// Non-blocking byte-stream transport.
///
/// Both methods are non-blocking and return the number of bytes transferred.
/// Returning `0` means no bytes were available (read) or the sink was full
/// (write). Callers must poll in a loop rather than block.
pub trait Transport {
    /// Read up to `buf.len()` bytes. Returns the number of bytes read.
    /// Returns `0` if no data is currently available.
    fn read(&mut self, buf: &mut [u8]) -> usize;

    /// Write up to `buf.len()` bytes. Returns the number of bytes written.
    /// May return less than `buf.len()` if the sink buffer is full.
    fn write(&mut self, buf: &[u8]) -> usize;
}
