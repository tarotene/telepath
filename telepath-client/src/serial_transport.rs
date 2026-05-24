use std::io::{self, Read, Write};
use std::time::Duration;

use crate::HostTransportExt;

/// Serial port adapter implementing `std::io::Read + Write` for use with
/// `TelepathClient`.
///
/// Wraps a `serialport::SerialPort` instance. The read timeout is configured
/// at open time via [`SerialTransport::new`]; callers can update it later via
/// [`HostTransportExt::set_read_deadline`].
pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
}

impl SerialTransport {
    /// Open the serial port at `path` with `baud_rate`.
    ///
    /// The port is opened with a 10-second initial read timeout. Callers
    /// should call [`HostTransportExt::set_read_deadline`] before discovery
    /// and again before each command call if a different timeout is needed.
    pub fn new(path: &str, baud_rate: u32) -> anyhow::Result<Self> {
        let port = serialport::new(path, baud_rate)
            .timeout(Duration::from_secs(10))
            .open()
            .map_err(|e| anyhow::anyhow!("Failed to open serial port '{}': {}", path, e))?;
        Ok(Self { port })
    }
}

impl HostTransportExt for SerialTransport {
    fn set_read_deadline(&mut self, timeout: Duration) {
        let _ = self.port.set_timeout(timeout);
    }

    fn clear_read_deadline(&mut self) {
        // Restore a generous blocking timeout; serialport has no true "infinite" mode.
        let _ = self.port.set_timeout(Duration::from_secs(3600));
    }
}

impl Read for SerialTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.port.read(buf)
    }
}

impl Write for SerialTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.port.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.port.flush()
    }
}
