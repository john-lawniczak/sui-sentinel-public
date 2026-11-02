//! Optional bearer protection for the combined monitor's read-only HTTP service.
use super::Registry;
use anyhow::{Result, ensure};
use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
};
use std::{net::SocketAddr, sync::Arc};

fn protected(registry: Arc<Registry>, token: Option<String>) -> Result<Router> {
    ensure!(
        token.as_ref().is_none_or(|t| t.len() >= 32
            && t.len() <= 1024
            && t.bytes().all(|b| b.is_ascii_graphic())),
        "read token must contain 32..1024 visible ASCII bytes"
    );
    Ok(super::registry::http::router(registry)
        .layer(middleware::from_fn_with_state(token, authorize)))
}
async fn authorize(
    State(token): State<Option<String>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.uri().path() != "/livez"
        && let Some(token) = token
    {
        let supplied = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        // Hashes have equal length; compare every byte rather than an early-exit string comparison.
        let expected = crate::storage::sha256_hex_of(token.as_bytes());
        let supplied = crate::storage::sha256_hex_of(supplied.as_bytes());
        let different = expected
            .bytes()
            .zip(supplied.bytes())
            .fold(0_u8, |d, (a, b)| d | (a ^ b));
        if different != 0 {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(next.run(request).await)
}
pub async fn serve_registry(
    registry: Arc<Registry>,
    listen: SocketAddr,
    token: Option<String>,
) -> Result<()> {
    let app = protected(registry, token)?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    #[tokio::test]
    async fn only_liveness_is_public_when_token_is_configured() {
        let registry = Arc::new(Registry::with_policy(
            std::env::temp_dir(),
            crate::monitoring::Policy::builtin().unwrap(),
        ));
        let token = "a".repeat(32);
        let app = protected(registry, Some(token.clone())).unwrap();
        for path in ["/metrics", "/status", "/readyz", "/report", "/unknown"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/livez")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
