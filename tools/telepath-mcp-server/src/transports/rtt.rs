// MIRROR of tools/telepath-shell/src/rtt_transport.rs.
// Follow-up: consolidate into a shared crate (see issue TBD).

use anyhow::Context;
use probe_rs::{
    rtt::{Rtt, ScanRegion},
    Session,
};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

/// RTT adapter implementing `std::io::Read + Write` for use with `TelepathClient`.
///
/// Owns a probe-rs `Session` and an `Rtt` instance. Each I/O call transiently
/// acquires a `Core` handle from the session — the expected probe-rs usage pattern.
///
/// Set a read deadline before calling `TelepathClient::call_raw` to prevent
/// indefinite blocking when the firmware does not respond.
pub struct RttTransport {
    session: Session,
    rtt: Rtt,
    core_index: usize,
    up_channel: usize,
    down_channel: usize,
    read_deadline: Option<Instant>,
}

impl RttTransport {
    /// Attach to RTT on `session` using the given channel indices.
    ///
    /// The channel layout matches `examples/nrf52840-ping`:
    /// - up 0 / (no down): firmware debug output
    /// - up 1 / down 1: Telepath RPC traffic
    pub fn new(
        mut session: Session,
        core_index: usize,
        up_channel: usize,
        down_channel: usize,
        control_block_addr: u64,
    ) -> anyhow::Result<Self> {
        let rtt = {
            let mut core = session.core(core_index).context("Failed to access core")?;
            Rtt::attach_region(&mut core, &ScanRegion::Exact(control_block_addr))
                .with_context(|| {
                    format!(
                        "Failed to attach to RTT at {:#010x}. Is the firmware running?",
                        control_block_addr
                    )
                })?
        };
        Ok(Self {
            session,
            rtt,
            core_index,
            up_channel,
            down_channel,
            read_deadline: None,
        })
    }

    /// Arm an absolute read deadline at `Instant::now() + timeout`.
    pub fn set_read_deadline(&mut self, timeout: Duration) {
        self.read_deadline = Some(Instant::now() + timeout);
    }

    #[allow(dead_code)]
    pub fn clear_read_deadline(&mut self) {
        self.read_deadline = None;
    }

    /// Drain RTT channel 0 (firmware debug output) to stderr. Non-blocking.
    pub fn drain_debug_logs(&mut self) {
        let mut buf = [0u8; 1024];
        let mut core = match self.session.core(self.core_index) {
            Ok(c) => c,
            Err(_) => return,
        };
        if let Some(ch0) = self.rtt.up_channel(0) {
            loop {
                let n = ch0.read(&mut core, &mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                eprint!("{}", String::from_utf8_lossy(&buf[..n]));
            }
        }
    }
}

/// Open the first available debug probe, attach to `chip`, and return an `RttTransport`.
///
/// Uses RTT channels up 1 / down 1 for Telepath RPC traffic (up 0 carries debug logs).
pub fn attach(chip: &str, control_block_addr: u64) -> anyhow::Result<RttTransport> {
    use probe_rs::{probe::list::Lister, Permissions};

    let lister = Lister::new();
    let probes = lister.list_all();
    anyhow::ensure!(
        !probes.is_empty(),
        "No debug probes found. Is the J-Link / CMSIS-DAP connected?"
    );
    let probe = probes
        .into_iter()
        .next()
        .unwrap()
        .open()
        .context("Failed to open debug probe")?;
    let session = probe
        .attach(chip, Permissions::default())
        .with_context(|| format!("Failed to attach to target '{chip}'"))?;
    RttTransport::new(session, 0, 1, 1, control_block_addr)
}

impl Read for RttTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // probe-rs RTT `read` is non-blocking and returns Ok(0) when no data is
        // available. Busy-loop with a 1ms sleep so that `read_exact` in
        // `TelepathClient::call_raw` does not mistake an empty read for EOF.
        loop {
            if let Some(deadline) = self.read_deadline {
                if Instant::now() > deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "RTT read deadline exceeded",
                    ));
                }
            }
            let core_index = self.core_index;
            let up_channel = self.up_channel;
            let mut core = self
                .session
                .core(core_index)
                .map_err(|e| io::Error::other(e.to_string()))?;
            let ch = self.rtt.up_channel(up_channel).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "RTT up channel not found")
            })?;
            let n = ch
                .read(&mut core, buf)
                .map_err(|e| io::Error::other(e.to_string()))?;
            if n > 0 {
                return Ok(n);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Write for RttTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let core_index = self.core_index;
        let down_channel = self.down_channel;
        let mut core = self
            .session
            .core(core_index)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let ch = self
            .rtt
            .down_channel(down_channel)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "RTT down channel not found"))?;
        ch.write(&mut core, buf)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
