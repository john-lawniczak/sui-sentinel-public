//! HTTP route wiring for the polling registry. Kept separate from
//! `Registry` itself (`mod.rs`) so the reload/state-machine logic can be
//! read and tested without wading through route handlers, and the route
//! surface can be read without the locking/reload internals.

use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    routing::get,
};
use time::OffsetDateTime;

use super::{Registry, Status};

pub(super) fn format_time(value: OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unavailable".to_owned())
}

fn status_response(status: &Status) -> ([(header::HeaderName, &'static str); 2], Vec<u8>) {
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::to_vec(status).unwrap_or_else(|_| b"{}".to_vec()),
    )
}

pub(crate) fn router(registry: Arc<Registry>) -> Router {
    Router::new()
        .route(
            "/metrics",
            get(|State(state): State<Arc<Registry>>| async move {
                let (presentation, _) = state.snapshot();
                let mut text = state.operational_metrics();
                if let Some(presentation) = presentation {
                    text.push_str(std::str::from_utf8(&presentation.metrics).unwrap_or(""));
                }
                (
                    [
                        (
                            header::CONTENT_TYPE,
                            "text/plain; version=0.0.4; charset=utf-8",
                        ),
                        (header::CACHE_CONTROL, "no-store"),
                    ],
                    text,
                )
            }),
        )
        .route(
            "/report",
            get(|State(state): State<Arc<Registry>>| async move {
                let (presentation, _) = state.snapshot();
                match presentation {
                    Some(presentation) => Ok((
                        [
                            (header::CONTENT_TYPE, "application/json"),
                            (header::CACHE_CONTROL, "no-store"),
                        ],
                        presentation.report.clone(),
                    )),
                    None => Err(StatusCode::SERVICE_UNAVAILABLE),
                }
            }),
        )
        // Liveness: this process is responsive. Never reflects report state -
        // a stuck collector must not also make the HTTP process look dead.
        .route("/livez", get(|| async { "alive\n" }))
        // Historical reports remain readable even when readiness is degraded.
        .route(
            "/readyz",
            get(|State(state): State<Arc<Registry>>| async move {
                let (_, status) = state.snapshot();
                let code = if status.ready {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                (code, status_response(&status))
            }),
        )
        .route(
            "/status",
            get(|State(state): State<Arc<Registry>>| async move {
                let (_, status) = state.snapshot();
                status_response(&status)
            }),
        )
        .with_state(registry)
}
