//! RTT-backed [`Transport`] implementation for the nRF52840-DK.
//!
//! Uses RTT channel 1 for both up (target→host) and down (host→target)
//! directions. Channel 0 is reserved for debug printing via `rprintln!`.

use rtt_target::{DownChannel, UpChannel};
use telepath_firmware::transport::Transport;

/// RTT channel 1 transport.
///
/// Wraps `rtt-target` up and down channels into the Telepath [`Transport`]
/// trait. Both `read` and `write` are non-blocking and return the actual
/// number of bytes transferred.
pub struct RttTransport {
    up: UpChannel,
    down: DownChannel,
}

impl RttTransport {
    /// Create a new transport from the given RTT up and down channels.
    pub fn new(up: UpChannel, down: DownChannel) -> Self {
        Self { up, down }
    }
}

impl Transport for RttTransport {
    /// Read bytes from the down channel (host→target, i.e. incoming requests).
    fn read(&mut self, buf: &mut [u8]) -> usize {
        self.down.read(buf)
    }

    /// Write bytes to the up channel (target→host, i.e. outgoing responses).
    fn write(&mut self, buf: &[u8]) -> usize {
        self.up.write(buf)
    }
}
