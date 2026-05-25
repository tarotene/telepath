use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use telepath_server::transport::Transport as FwTransport;
use telepath_wire::framing::MAX_FRAME_SIZE;

pub struct FwSide {
    pub rx: Receiver<u8>,
    pub tx: SyncSender<u8>,
}

pub struct HostSide {
    pub rx: Receiver<u8>,
    pub tx: SyncSender<u8>,
}

pub fn make_pair() -> (FwSide, HostSide) {
    let cap = MAX_FRAME_SIZE * 4;
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

impl FwTransport for FwSide {
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
        let mut n = 0;
        for &b in buf {
            match self.tx.try_send(b) {
                Ok(()) => n += 1,
                Err(_) => return n,
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

/// RAII guard that stops and joins the firmware thread on drop.
pub struct FwGuard {
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for FwGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Spawn a firmware thread running `server_factory(fw_side, running)` and return an RAII guard.
///
/// The `running` flag is `true` while the guard is alive and set to `false` on drop.
/// The factory must poll until `running.load(Ordering::Acquire)` returns `false`.
pub fn spawn_fw<F>(fw_side: FwSide, server_factory: F) -> FwGuard
where
    F: FnOnce(FwSide, Arc<AtomicBool>) + Send + 'static,
{
    let running = Arc::new(AtomicBool::new(true));
    let running_fw = Arc::clone(&running);
    let handle = thread::spawn(move || {
        server_factory(fw_side, running_fw);
    });
    FwGuard {
        running,
        handle: Some(handle),
    }
}
