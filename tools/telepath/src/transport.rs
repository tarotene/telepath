#[cfg(not(any(feature = "rtt", feature = "serial")))]
compile_error!(
    "telepath requires at least one transport feature: \
     enable `rtt` and/or `serial`"
);

use super::cli::{TransportArgs, TransportKind};
use std::io::{self, Read, Write};
use std::time::Duration;
use telepath_client::HostTransportExt;

pub enum AnyTransport {
    #[cfg(feature = "rtt")]
    Rtt(telepath_client::rtt_transport::RttTransport),
    #[cfg(feature = "serial")]
    Serial(telepath_client::serial_transport::SerialTransport),
}

impl Read for AnyTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(feature = "rtt")]
            AnyTransport::Rtt(t) => t.read(buf),
            #[cfg(feature = "serial")]
            AnyTransport::Serial(t) => t.read(buf),
        }
    }
}

impl Write for AnyTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(feature = "rtt")]
            AnyTransport::Rtt(t) => t.write(buf),
            #[cfg(feature = "serial")]
            AnyTransport::Serial(t) => t.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(feature = "rtt")]
            AnyTransport::Rtt(t) => t.flush(),
            #[cfg(feature = "serial")]
            AnyTransport::Serial(t) => t.flush(),
        }
    }
}

impl HostTransportExt for AnyTransport {
    fn set_read_deadline(&mut self, timeout: Duration) {
        match self {
            #[cfg(feature = "rtt")]
            AnyTransport::Rtt(t) => t.set_read_deadline(timeout),
            #[cfg(feature = "serial")]
            AnyTransport::Serial(t) => t.set_read_deadline(timeout),
        }
    }

    fn clear_read_deadline(&mut self) {
        match self {
            #[cfg(feature = "rtt")]
            AnyTransport::Rtt(t) => t.clear_read_deadline(),
            #[cfg(feature = "serial")]
            AnyTransport::Serial(t) => t.clear_read_deadline(),
        }
    }

    fn drain_debug_logs(&mut self, sink: &mut dyn Write) {
        match self {
            #[cfg(feature = "rtt")]
            AnyTransport::Rtt(t) => t.drain_debug_logs(sink),
            #[cfg(feature = "serial")]
            AnyTransport::Serial(t) => t.drain_debug_logs(sink),
        }
    }

    fn drain_rpc_rx(&mut self) {
        match self {
            #[cfg(feature = "rtt")]
            AnyTransport::Rtt(t) => t.drain_rpc_rx(),
            #[cfg(feature = "serial")]
            AnyTransport::Serial(t) => t.drain_rpc_rx(),
        }
    }
}

pub fn build_transport(args: &TransportArgs) -> anyhow::Result<AnyTransport> {
    // Infer serial when --port is given without an explicit --transport serial,
    // so `telepath shell --port /dev/ttyACM0` works without extra flags.
    let effective = match (&args.transport, &args.port) {
        (TransportKind::Rtt, Some(_)) => TransportKind::Serial,
        _ => args.transport,
    };
    match effective {
        TransportKind::Rtt => build_rtt(args),
        TransportKind::Serial => build_serial(args),
    }
}

#[cfg(feature = "rtt")]
fn build_rtt(args: &TransportArgs) -> anyhow::Result<AnyTransport> {
    use probe_rs::{probe::list::Lister, Permissions};
    use std::time::Instant;

    let rtt_timing = std::env::var_os("TELEPATH_RTT_TIMING").is_some();

    let lister = Lister::new();
    let probes = lister.list_all();
    if probes.is_empty() {
        anyhow::bail!("No debug probes found. Is the J-Link / CMSIS-DAP connected?");
    }

    let t_probe_open = Instant::now();
    let probe = probes
        .into_iter()
        .next()
        .unwrap()
        .open()
        .map_err(|e| anyhow::anyhow!("Failed to open debug probe: {e}"))?;
    if rtt_timing {
        eprintln!(
            "[telepath:rtt-timing] probe.open elapsed={:?}",
            t_probe_open.elapsed()
        );
    }

    let t_session = Instant::now();
    let session = probe
        .attach(&args.chip, Permissions::default())
        .map_err(|e| anyhow::anyhow!("Failed to attach to target '{}': {e}", args.chip))?;
    if rtt_timing {
        eprintln!(
            "[telepath:rtt-timing] probe.attach({}) elapsed={:?}",
            args.chip,
            t_session.elapsed()
        );
    }

    let transport = telepath_client::rtt_transport::RttTransport::new(
        session,
        0,
        1,
        1,
        args.rtt_control_block_addr,
        !args.no_reset,
    )?;
    Ok(AnyTransport::Rtt(transport))
}

#[cfg(not(feature = "rtt"))]
fn build_rtt(_args: &TransportArgs) -> anyhow::Result<AnyTransport> {
    anyhow::bail!("RTT transport not compiled in; rebuild with --features rtt")
}

#[cfg(feature = "serial")]
fn build_serial(args: &TransportArgs) -> anyhow::Result<AnyTransport> {
    let port = args
        .port
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--port is required for serial transport"))?;
    let transport = telepath_client::serial_transport::SerialTransport::new(port, args.baud)?;
    Ok(AnyTransport::Serial(transport))
}

#[cfg(not(feature = "serial"))]
fn build_serial(_args: &TransportArgs) -> anyhow::Result<AnyTransport> {
    anyhow::bail!("Serial transport not compiled in; rebuild with --features serial")
}
