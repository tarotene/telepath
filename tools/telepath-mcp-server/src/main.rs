use clap::Parser;
use rmcp::ServiceExt;

#[derive(clap::ValueEnum, Clone, Debug)]
enum Transport {
    Loopback,
    Rtt,
}

#[derive(Parser)]
#[command(
    name = "telepath-mcp-server",
    about = "Exposes Telepath commands as MCP tools"
)]
struct Cli {
    #[arg(long, value_enum, default_value_t = Transport::Loopback, help = "Transport backend")]
    transport: Transport,

    /// Target chip name (required for --transport rtt).
    #[arg(long, default_value = "nRF52840_xxAA")]
    chip: String,

    /// SEGGER RTT control block address in hex (e.g. 0x20000000).
    #[arg(
        long,
        value_parser = parse_hex_u64,
        env = "TELEPATH_RTT_CONTROL_BLOCK_ADDR",
        default_value = "0x20000000"
    )]
    rtt_control_block_addr: u64,

    /// Disable automatic chip reset when the RTT control block is not found on attach.
    #[arg(long)]
    no_reset: bool,
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(digits, 16).map_err(|e| format!("invalid hex u64 '{s}': {e}"))
}

// ── demo commands (loopback mode only) ───────────────────────────────────────

#[cfg(feature = "loopback")]
mod loopback {
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use telepath_server::{command, TelepathServer};
    use telepath_wire::framing::MAX_FRAME_SIZE;

    #[command]
    fn ping() -> u32 {
        0xDEAD_BEEF
    }

    pub struct FwSide {
        rx: Receiver<u8>,
        tx: SyncSender<u8>,
    }

    pub struct HostSide {
        rx: Receiver<u8>,
        tx: SyncSender<u8>,
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

    impl telepath_server::transport::Transport for FwSide {
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

    pub fn run_server(fw_side: FwSide) {
        let mut server = TelepathServer::<_, 512>::new(fw_side, telepath_server::commands());
        loop {
            server.poll();
            std::thread::yield_now();
        }
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.transport {
        Transport::Loopback => {
            #[cfg(not(feature = "loopback"))]
            anyhow::bail!(
                "loopback transport requested but this binary was built without the `loopback` feature"
            );

            #[cfg(feature = "loopback")]
            {
                let (fw_side, host_side) = loopback::make_pair();
                std::thread::spawn(move || loopback::run_server(fw_side));
                let client = telepath_client::TelepathClient::new(host_side);
                let mcp_server = telepath_mcp_server::server::TelepathMcpServer::build(client)
                    .map_err(|e| anyhow::anyhow!("discover failed: {e:?}"))?;
                let running = mcp_server.serve(rmcp::transport::io::stdio()).await?;
                running.waiting().await?;
            }
        }

        Transport::Rtt => {
            #[cfg(not(feature = "rtt"))]
            anyhow::bail!(
                "RTT transport requested but this binary was built without the `rtt` feature"
            );

            #[cfg(feature = "rtt")]
            {
                use probe_rs::{probe::list::Lister, Permissions};
                use std::time::Duration;
                use telepath_mcp_server::rtt_transport::RttTransport;

                let lister = Lister::new();
                let probes = lister.list_all();
                if probes.is_empty() {
                    anyhow::bail!(
                        "No debug probes found. Is the J-Link / CMSIS-DAP connected?"
                    );
                }
                let probe = probes
                    .into_iter()
                    .next()
                    .unwrap()
                    .open()
                    .map_err(|e| anyhow::anyhow!("Failed to open debug probe: {e}"))?;
                let session = probe
                    .attach(&cli.chip, Permissions::default())
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to attach to target '{}': {e}", cli.chip)
                    })?;

                let mut transport = RttTransport::new(
                    session,
                    0,
                    1,
                    1,
                    cli.rtt_control_block_addr,
                    !cli.no_reset,
                )?;
                // Drain firmware boot logs to stderr — stdout is reserved for MCP JSON-RPC.
                transport.drain_debug_logs(&mut std::io::stderr());
                transport.set_read_deadline(Duration::from_secs(10));

                let client = telepath_client::TelepathClient::new(transport);
                let mcp_server = telepath_mcp_server::server::TelepathMcpServer::build(client)
                    .map_err(|e| anyhow::anyhow!("discover failed: {e:?}"))?;
                let running = mcp_server.serve(rmcp::transport::io::stdio()).await?;
                running.waiting().await?;
            }
        }
    }

    Ok(())
}
