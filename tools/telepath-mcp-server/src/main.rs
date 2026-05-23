use clap::Parser;

#[derive(Parser)]
#[command(name = "telepath-mcp-server")]
#[command(about = "MCP server exposing Telepath commands as MCP tools")]
struct Cli {
    #[arg(long, default_value = "loopback")]
    transport: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    let _cli = Cli::parse();
    todo!("transport setup + rmcp serve")
}
