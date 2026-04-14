//! `GET /metrics` — Prometheus scrape endpoint, loopback-only.

use axum::{
    extract::{ConnectInfo, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::net::SocketAddr;

use crate::routes::AppState;

async fn metrics(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !addr.ip().is_loopback() {
        return (StatusCode::FORBIDDEN, "metrics are loopback-only").into_response();
    }
    let body = state.metrics.render();
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
        .into_response()
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/metrics", get(metrics))
}
