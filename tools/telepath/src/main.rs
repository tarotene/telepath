mod cli;
mod cmd;
mod transport;

use clap::Parser;
use cli::{Cli, Command};
use telepath_client::TelepathClient;
use transport::build_transport;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        #[cfg(feature = "shell")]
        Command::Shell(args) => {
            let transport = build_transport(&args.transport)?;
            let client = TelepathClient::new(transport);
            cmd::shell::run(args, client)
        }
        #[cfg(feature = "mcp")]
        Command::Mcp(args) => run_mcp(args),
    }
}

#[cfg(feature = "mcp")]
fn run_mcp(args: &cli::McpArgs) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    use telepath_client::HostTransportExt;
    let mut transport = build_transport(&args.transport)?;
    transport.drain_debug_logs(&mut std::io::stderr());
    transport.drain_rpc_rx();
    transport.set_read_deadline(std::time::Duration::from_secs(10));

    let client = TelepathClient::new(transport);

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(cmd::mcp::run(args, client))
}
