use super::Sentinel;
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{StatusCode, header},
    routing::{get, post},
};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    kind: String,
    request: crate::query::Request,
}
pub(super) fn routes(server: Sentinel) -> Router {
    Router::new()
        .route("/",get(||async{([(header::CONTENT_TYPE,"text/html; charset=utf-8"),(header::CONTENT_SECURITY_POLICY,"default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"),(header::CACHE_CONTROL,"no-store")],include_str!("operator.html"))}))
        .route("/operator.js",get(||async{([(header::CONTENT_TYPE,"text/javascript")],include_str!("operator.js"))}))
        .route("/operator.css",get(||async{([(header::CONTENT_TYPE,"text/css")],include_str!("operator.css"))}))
        .route("/query",post(query))
        .layer(DefaultBodyLimit::max(16*1024)).with_state(server)
}
async fn query(
    State(server): State<Sentinel>,
    body: Bytes,
) -> Result<([(header::HeaderName, &'static str); 1], String), StatusCode> {
    let input: Input = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let value = server.queries.execute(&input.kind, input.request).await;
    Ok((
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    ))
}
