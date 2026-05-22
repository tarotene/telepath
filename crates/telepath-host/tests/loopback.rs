//! Integration test: full COBS+postcard round-trip over an in-memory loopback.
//!
//! Mirrors the `examples/host-emulator` scenario but as a `cargo test`
//! so that regressions surface with clear failure output rather than
//! a timed-out process.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;

use telepath_firmware::transport::Transport;
use telepath_firmware::{CommandMetadata, DispatchError, TelepathServer};
use telepath_host::TelepathClient;
use telepath_wire::framing::MAX_FRAME_SIZE;

// ---------------------------------------------------------------------------
// Inline loopback transport (not shared with the binary crate)
// ---------------------------------------------------------------------------

struct FwSide {
    rx: Receiver<u8>,
    tx: SyncSender<u8>,
}
struct HostSide {
    rx: Receiver<u8>,
    tx: SyncSender<u8>,
}

fn make_pair() -> (FwSide, HostSide) {
    let cap = MAX_FRAME_SIZE * 2;
    let (h2f_tx, h2f_rx) = sync_channel::<u8>(cap);
    let (f2h_tx, f2h_rx) = sync_channel::<u8>(cap);
    (
        FwSide {
            rx: h2f_rx,
            tx: f2h_tx,
        },
        HostSide {
            rx: f2h_rx,
            tx: h2f_tx,
        },
    )
}

impl Transport for FwSide {
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
        use std::sync::mpsc::TrySendError;
        let mut n = 0;
        for &b in buf {
            match self.tx.try_send(b) {
                Ok(()) => n += 1,
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => return n,
            }
        }
        n
    }
}

impl std::io::Read for HostSide {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let first = self.rx.recv().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "fw disconnected")
        })?;
        buf[0] = first;
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

impl std::io::Write for HostSide {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &b in buf {
            self.tx.send(b).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "fw disconnected")
            })?;
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

fn ping_shim(_input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError> {
    let val: u32 = 0xDEAD_BEEF;
    let written = postcard::to_slice(&val, output).map_err(|_| DispatchError::SerializeError)?;
    Ok(written.len())
}

fn noop_schema(_out: &mut [u8]) -> Result<usize, ()> {
    Ok(0)
}

static COMMANDS: [CommandMetadata; 1] = [CommandMetadata {
    name: "ping",
    id: 0x0001,
    invoke: ping_shim,
    args_schema: noop_schema,
    ret_schema: noop_schema,
}];

#[test]
fn ping_round_trip_over_loopback() {
    let (fw_t, host_t) = make_pair();
    let running = Arc::new(AtomicBool::new(true));

    let running_fw = Arc::clone(&running);
    let fw_handle = thread::spawn(move || {
        let mut server = TelepathServer::<_, 512>::new(fw_t, &COMMANDS);
        while running_fw.load(Ordering::Acquire) {
            server.poll();
            thread::yield_now();
        }
    });

    let mut client = TelepathClient::new(host_t);
    let payload = client.call_raw(0x0001, &[]).expect("call_raw failed");
    let val: u32 = postcard::from_bytes(&payload).expect("deserialize failed");

    running.store(false, Ordering::Release);
    fw_handle.join().expect("fw thread panicked");

    assert_eq!(val, 0xDEAD_BEEF);
}
