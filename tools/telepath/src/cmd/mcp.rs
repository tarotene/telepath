use super::super::cli::McpArgs;
use super::super::transport::AnyTransport;
use rmcp::ServiceExt;
use telepath_client::TelepathClient;

pub async fn run(_args: &McpArgs, client: TelepathClient<AnyTransport>) -> anyhow::Result<()> {
    let mcp_server = telepath::TelepathMcpServer::build(client)
        .map_err(|e| anyhow::anyhow!("discover failed: {e:?}"))?;
    let running = mcp_server.serve(rmcp::transport::io::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
