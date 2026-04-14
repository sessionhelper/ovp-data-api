//! WebSocket event bus.
//!
//! Protocol (server expects JSON text frames):
//!
//! ```json
//! { "type": "subscribe",   "events": ["chunk_uploaded","segment_created"], "filter": { "guild_id": 123 } }
//! { "type": "unsubscribe", "events": ["segment_created"] }
//! ```
//!
//! Server replies:
//!
//! ```json
//! { "type": "subscribed", "active_filters": [...] }
//! { "type": "<event_name>", "at_ts": "...", "data": { ... } }
//! ```
//!
//! Delivery semantics:
//! - Bounded per-subscriber queue (`WS_QUEUE_DEPTH`, default 64). Overflow
//!   drops the oldest event, emits a WARN log, and increments a metric.
//! - Ping every 30s; 10s grace before drop.

mod filter;
mod queue;
mod session;

pub use filter::{Filter, Subscription};

use axum::{
    extract::{Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;

use crate::auth::hash_token;
use crate::db::service_sessions;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn upgrade(
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // Bearer auth supports both the Authorization header and a `?token=`
    // query param; browser WS clients can't set custom headers on upgrade.
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or(q.token);
    let token = match bearer {
        Some(t) => t,
        None => {
            return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
        }
    };

    let hash = hash_token(&token);
    let name = match service_sessions::lookup_alive(&state.pool, &hash, state.heartbeat_reap).await
    {
        Ok(Some(n)) => n,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "invalid token").into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "auth lookup failed").into_response(),
    };

    ws.on_upgrade(move |socket| session::run(socket, state, name))
        .into_response()
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/internal/ws", get(upgrade))
}
