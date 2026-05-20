//! In-memory byte-channel transport pair.
//!
//! One end implements `telepath_firmware::transport::Transport` (non-blocking),
//! the other implements `std::io::Read + Write` (blocking on first byte).

use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

use telepath_firmware::transport::Transport;
use telepath_wire::framing::MAX_FRAME_SIZE;

/// The firmware-side end of the loopback pair.
///
/// Both `read` and `write` are non-blocking and return the number of bytes
/// transferred, matching the `Transport` contract exactly.
pub struct FwSideTransport {
    rx: Receiver<u8>,   // host → fw
    tx: SyncSender<u8>, // fw  → host
}

/// The host-side end of the loopback pair.
///
/// `Read::read` blocks on the first byte (via `recv`) then drains additional
/// bytes non-blockingly, matching the semantics `TelepathClient::call_raw`
/// expects from its transport.
pub struct HostSideTransport {
    rx: Receiver<u8>,   // fw   → host
    tx: SyncSender<u8>, // host → fw
}

/// Create a connected (`FwSideTransport`, `HostSideTransport`) pair.
pub fn make_pair() -> (FwSideTransport, HostSideTransport) {
    let cap = MAX_FRAME_SIZE * 2;
    let (h2f_tx, h2f_rx) = sync_channel::<u8>(cap);
    let (f2h_tx, f2h_rx) = sync_channel::<u8>(cap);
    (
        FwSideTransport {
            rx: h2f_rx,
            tx: f2h_tx,
        },
        HostSideTransport {
            rx: f2h_rx,
            tx: h2f_tx,
        },
    )
}

impl Transport for FwSideTransport {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while n < buf.len() {
            match self.rx.try_recv() {
                Ok(b) => {
                    buf[n] = b;
                    n += 1;
                }
                Err(_) => break,
            }
        }
        n
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        for &b in buf {
            if self.tx.send(b).is_err() {
                return 0;
            }
        }
        buf.len()
    }
}

impl std::io::Read for HostSideTransport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // Block until the firmware sends the first byte of the response frame.
        let first = self.rx.recv().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "fw thread disconnected")
        })?;
        buf[0] = first;
        // Drain any additional bytes that are already buffered.
        let mut n = 1;
        while n < buf.len() {
            match self.rx.try_recv() {
                Ok(b) => {
                    buf[n] = b;
                    n += 1;
                }
                Err(_) => break,
            }
        }
        Ok(n)
    }
}

impl std::io::Write for HostSideTransport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &b in buf {
            self.tx.send(b).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "fw thread disconnected")
            })?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
