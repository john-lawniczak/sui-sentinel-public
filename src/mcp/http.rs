//! Private Streamable HTTP transport. Shared-token authentication is not OAuth discovery.
use super::Sentinel;
use anyhow::{Result, ensure};
use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
#[derive(Clone)]
struct Guard {
    hash: String,
    hosts: Vec<String>,
    slots: Arc<tokio::sync::Semaphore>,
}
pub fn router(server: Sentinel, token: &str, hosts: Vec<String>) -> Result<Router> {
    ensure!(
        (32..=1024).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_graphic()),
        "MCP token must contain 32..1024 visible ASCII bytes"
    );
    ensure!(
        !hosts.is_empty()
            && hosts.len() <= 16
            && hosts.iter().all(|s| !s.is_empty() && !s.contains('*')),
        "explicit MCP host allowlist required"
    );
    let mut config = StreamableHttpServerConfig::default().enforce_origin_validation();
    config.legacy_session_mode = false;
    config.json_response = true;
    config.max_request_body_bytes = 16 * 1024;
    config.allowed_hosts = hosts.clone();
    let operator = super::operator::routes(server.clone());
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Ok(Router::new()
        .nest_service("/mcp", service)
        .merge(operator)
        .layer(middleware::from_fn_with_state(
            Guard {
                hash: crate::storage::sha256_hex_of(token.as_bytes()),
                hosts,
                slots: Arc::new(tokio::sync::Semaphore::new(8)),
            },
            guard,
        )))
}
async fn guard(
    State(state): State<Guard>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !state.hosts.iter().any(|h| h == host) {
        return Err(StatusCode::FORBIDDEN);
    }
    if matches!(request.uri().path(), "/" | "/operator.js" | "/operator.css") {
        return Ok(next.run(request).await);
    }
    if let Some(origin) = request.headers().get("origin") {
        let own = origin
            .to_str()
            .is_ok_and(|o| o == format!("http://{host}") || o == format!("https://{host}"));
        if request.uri().path() != "/query" || !own {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    let token = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    let actual = crate::storage::sha256_hex_of(token.as_bytes());
    if actual
        .bytes()
        .zip(state.hash.bytes())
        .fold(0u8, |v, (a, b)| v | (a ^ b))
        != 0
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _permit = state
        .slots
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let mut response = tokio::time::timeout(Duration::from_secs(5), next.run(request))
        .await
        .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?;
    response
        .headers_mut()
        .insert("Cache-Control", "no-store".parse().unwrap());
    Ok(response)
}
pub async fn serve(
    server: Sentinel,
    listen: SocketAddr,
    token: &str,
    hosts: Vec<String>,
) -> Result<()> {
    ensure!(
        listen.ip().is_loopback(),
        "MCP HTTP binds loopback only; use an authenticated TLS tunnel/gateway for remote access"
    );
    let app = router(server, token, hosts)?;
    axum::serve(tokio::net::TcpListener::bind(listen).await?, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
