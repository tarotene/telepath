#[cfg(all(feature = "rtt", feature = "serial"))]
compile_error!("telepath-mcp-server: enable exactly one of `rtt`/`serial`; use --no-default-features --features serial for serial-only.");

#[cfg(not(any(feature = "rtt", feature = "serial")))]
compile_error!("telepath-mcp-server: at least one transport feature must be enabled (`rtt` or `serial`).");

use clap::Parser;
use rmcp::ServiceExt;
use telepath_client::HostTransportExt;

#[cfg(feature = "rtt")]
use telepath_client::rtt_transport::RttTransport;
#[cfg(feature = "serial")]
use telepath_client::serial_transport::SerialTransport;

#[derive(Parser)]
#[command(
    name = "telepath-mcp-server",
    about = "Exposes Telepath commands as MCP tools"
)]
struct Cli {
    #[cfg(feature = "rtt")]
    /// Target chip name.
    #[arg(long, default_value = "nRF52840_xxAA")]
    chip: String,

    #[cfg(feature = "rtt")]
    /// SEGGER RTT control block address in hex (e.g. 0x20000000).
    #[arg(
        long,
        value_parser = parse_hex_u64,
        env = "TELEPATH_RTT_CONTROL_BLOCK_ADDR",
        default_value = "0x20000000"
    )]
    rtt_control_block_addr: u64,

    #[cfg(feature = "rtt")]
    /// Disable automatic chip reset when the RTT control block is not found on attach.
    #[arg(long)]
    no_reset: bool,

    #[cfg(feature = "serial")]
    /// Serial port path (e.g. /dev/ttyUSB0, /dev/ttyACM0, COM3).
    #[arg(long)]
    port: String,

    #[cfg(feature = "serial")]
    /// Serial baud rate.
    #[arg(long, default_value = "115200")]
    baud: u32,
}

#[cfg(feature = "rtt")]
fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(digits, 16).map_err(|e| format!("invalid hex u64 '{s}': {e}"))
}

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

    #[cfg(feature = "rtt")]
    {
        use probe_rs::{probe::list::Lister, Permissions};
        use std::time::Duration;

        let lister = Lister::new();
        let probes = lister.list_all();
        if probes.is_empty() {
            anyhow::bail!("No debug probes found. Is the J-Link / CMSIS-DAP connected?");
        }
        let probe = probes
            .into_iter()
            .next()
            .unwrap()
            .open()
            .map_err(|e| anyhow::anyhow!("Failed to open debug probe: {e}"))?;
        let session = probe
            .attach(&cli.chip, Permissions::default())
            .map_err(|e| anyhow::anyhow!("Failed to attach to target '{}': {e}", cli.chip))?;

        let mut transport =
            RttTransport::new(session, 0, 1, 1, cli.rtt_control_block_addr, !cli.no_reset)?;
        // Drain firmware boot logs to stderr — stdout is reserved for MCP JSON-RPC.
        transport.drain_debug_logs(&mut std::io::stderr());
        transport.set_read_deadline(Duration::from_secs(10));

        let client = telepath_client::TelepathClient::new(transport);
        let mcp_server = telepath_mcp_server::server::TelepathMcpServer::build(client)
            .map_err(|e| anyhow::anyhow!("discover failed: {e:?}"))?;
        let running = mcp_server.serve(rmcp::transport::io::stdio()).await?;
        running.waiting().await?;
    }

    #[cfg(feature = "serial")]
    {
        use std::time::Duration;

        let mut transport = SerialTransport::new(&cli.port, cli.baud)?;
        transport.set_read_deadline(Duration::from_secs(10));

        let client = telepath_client::TelepathClient::new(transport);
        let mcp_server = telepath_mcp_server::server::TelepathMcpServer::build(client)
            .map_err(|e| anyhow::anyhow!("discover failed: {e:?}"))?;
        let running = mcp_server.serve(rmcp::transport::io::stdio()).await?;
        running.waiting().await?;
    }

    Ok(())
}
