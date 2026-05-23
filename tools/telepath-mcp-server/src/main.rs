use anyhow::Context;
use clap::Parser;
use rmcp::ServiceExt;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use telepath_mcp_server::transports::{parse_transport, TransportSpec};
use telepath_server::{command, TelepathServer};
use telepath_wire::framing::MAX_FRAME_SIZE;

#[derive(Parser)]
#[command(name = "telepath-mcp-server", about = "Exposes Telepath commands as MCP tools")]
struct Cli {
    /// Transport backend: loopback | rtt | serial:<path>
    #[arg(long, value_parser = parse_transport, default_value = "loopback")]
    transport: TransportSpec,

    /// Target chip name. Used when --transport rtt.
    #[arg(long, default_value = "nRF52840_xxAA")]
    chip: String,

    /// SEGGER RTT control block address in hex (e.g. 0x20000000).
    /// Falls back to env var TELEPATH_RTT_CONTROL_BLOCK_ADDR, then 0x20000000.
    #[arg(
        long,
        value_parser = parse_hex_u64,
        env = "TELEPATH_RTT_CONTROL_BLOCK_ADDR",
        default_value = "0x20000000"
    )]
    rtt_control_block_addr: u64,

    /// Baud rate. Used when --transport serial:<path>.
    #[arg(long, default_value = "115200")]
    baud: u32,
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(digits, 16).map_err(|e| format!("invalid hex u64 '{s}': {e}"))
}

// ── demo commands (loopback mode) ────────────────────────────────────────────

#[command]
fn ping() -> u32 {
    0xDEAD_BEEF
}

// ── loopback transport pair ───────────────────────────────────────────────────

struct FwSide {
    rx: Receiver<u8>,
    tx: SyncSender<u8>,
}

pub struct HostSide {
    rx: Receiver<u8>,
    tx: SyncSender<u8>,
}

fn make_pair() -> (FwSide, HostSide) {
    let cap = MAX_FRAME_SIZE * 4;
    let (h2f_tx, h2f_rx) = sync_channel::<u8>(cap);
    let (f2h_tx, f2h_rx) = sync_channel::<u8>(cap);
    (
        FwSide { rx: h2f_rx, tx: f2h_tx },
        HostSide { rx: f2h_rx, tx: h2f_tx },
    )
}

impl telepath_server::transport::Transport for FwSide {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while n < buf.len() {
            match self.rx.try_recv() {
                Ok(b) => { buf[n] = b; n += 1; }
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
        if buf.is_empty() { return Ok(0); }
        let first = self.rx.recv().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "fw disconnected")
        })?;
        buf[0] = first;
        let mut n = 1;
        while n < buf.len() {
            match self.rx.try_recv() {
                Ok(b) => { buf[n] = b; n += 1; }
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
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

// ── helpers ───────────────────────────────────────────────────────────────────

async fn serve_mcp<T>(client: telepath_client::TelepathClient<T>) -> anyhow::Result<()>
where
    T: std::io::Read + std::io::Write + Send + 'static,
{
    let mcp_server = telepath_mcp_server::server::TelepathMcpServer::build(client)
        .map_err(|e| anyhow::anyhow!("discover failed: {e:?}"))?;
    let running = mcp_server.serve(rmcp::transport::io::stdio()).await?;
    running.waiting().await?;
    Ok(())
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
        TransportSpec::Loopback => {
            let (fw_side, host_side) = make_pair();
            std::thread::spawn(move || {
                let mut server =
                    TelepathServer::<_, 512>::new(fw_side, telepath_server::commands());
                loop {
                    server.poll();
                    std::thread::yield_now();
                }
            });
            let client = telepath_client::TelepathClient::new(host_side);
            serve_mcp(client).await?;
        }
        TransportSpec::Rtt => {
            let transport =
                telepath_mcp_server::transports::rtt::attach(&cli.chip, cli.rtt_control_block_addr)
                    .context("Failed to attach RTT transport")?;
            let client = telepath_client::TelepathClient::new(transport);
            serve_mcp(client).await?;
        }
        TransportSpec::Serial(path) => {
            let transport =
                telepath_mcp_server::transports::serial::SerialTransport::open(&path, cli.baud)
                    .context("Failed to open serial transport")?;
            let client = telepath_client::TelepathClient::new(transport);
            serve_mcp(client).await?;
        }
    }

    Ok(())
}
