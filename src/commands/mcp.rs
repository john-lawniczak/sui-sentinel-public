use anyhow::Result;
use clap::Args;
use rmcp::ServiceExt;
#[derive(Args)]
pub struct McpArgs {
    #[arg(long, env = "SENTINEL_QUERY_DATABASE_URL", hide_env_values = true)]
    pub database_url: String,
    /// Explicit authorized scope, repeat for each permitted projection/incident scope.
    #[arg(long, required = true)]
    pub scope: Vec<String>,
    /// Omit for local stdio. HTTP is restricted to loopback behind a TLS tunnel.
    #[arg(long)]
    pub listen: Option<std::net::SocketAddr>,
    #[arg(long, env = "SENTINEL_MCP_TOKEN", hide_env_values = true)]
    pub token: Option<String>,
    #[arg(long)]
    pub allowed_host: Vec<String>,
}
pub async fn run(args: McpArgs) -> Result<()> {
    let server = sui_sentinel::mcp::Sentinel {
        queries: sui_sentinel::query::Queries::connect(&args.database_url, args.scope).await?,
    };
    if let Some(listen) = args.listen {
        let token = args
            .token
            .ok_or_else(|| anyhow::anyhow!("SENTINEL_MCP_TOKEN required for HTTP"))?;
        sui_sentinel::mcp::http::serve(server, listen, &token, args.allowed_host).await
    } else {
        let transport = rmcp::transport::async_rw::AsyncRwTransport::new(
            tokio::io::stdin(),
            tokio::io::stdout(),
        )
        .with_max_line_length(16 * 1024);
        server.serve(transport).await?.waiting().await?;
        Ok(())
    }
}
