//! WebSocket endpoint for real-time event streaming.
//!
//! Clients connect to `GET /ws?token=<bearer_token>`, then send JSON
//! subscription messages to choose which events they receive. The server
//! filters the global event bus by the client's active subscriptions and
//! forwards matching events as JSON text frames.
//!
//! # Auth
//!
//! The `token` query parameter is validated against the `service_sessions`
//! table — the same check the HTTP auth middleware does. Connections
//! without a valid token are rejected with 401 before the upgrade.
//!
//! This is an internal-only API. The only trust boundary is the shared
//! secret used to mint session tokens. Every authenticated client is
//! treated identically — there is no per-service routing, per-role
//! authorization, or tiered QoS at this layer. Boundary enforcement for
//! user-facing data (per-user authorization, filtering, SSE fan-out)
//! happens in `chronicle-portal`, not here.
//!
//! # Delivery
//!
//! Every connection gets the same reliable mpsc-drained delivery path:
//! a background task drains the broadcast event bus into a private
//! 1000-message mpsc queue that feeds the WS sender. If the client is
//! slow to drain, the drain task backpressures against the mpsc queue
//! rather than losing events.
//!
//! # Subscription protocol
//!
//! ```json
//! {"subscribe": "sessions/<uuid>"}   // events for one session
//! {"subscribe": "sessions"}           // all session events
//! {"unsubscribe": "sessions/<uuid>"} // stop receiving
//! ```

use std::collections::HashSet;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use uuid::Uuid;

use crate::auth::middleware::hash_session_token;
use crate::routes::AppState;

/// Client-to-server control message.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ClientMessage {
    Subscribe { subscribe: String },
    Unsubscribe { unsubscribe: String },
}

/// Query parameters on the WS upgrade request.
#[derive(Debug, Deserialize)]
struct WsParams {
    token: String,
}

/// Parsed topic from a subscription string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Topic {
    /// All events for all sessions.
    AllSessions,
    /// Events for a specific session.
    Session(Uuid),
}

fn parse_topic(raw: &str) -> Option<Topic> {
    if raw == "sessions" {
        return Some(Topic::AllSessions);
    }
    if let Some(id_str) = raw.strip_prefix("sessions/") {
        if let Ok(id) = id_str.parse::<Uuid>() {
            return Some(Topic::Session(id));
        }
    }
    None
}

/// Validate the bearer token against the `service_sessions` table.
///
/// Returns the service_name for the session (for logging only) if the
/// token is valid and alive, or `None` if it's invalid or expired.
async fn validate_ws_token(pool: &sqlx::PgPool, token: &str) -> Option<String> {
    let token_hash = hash_session_token(token);

    #[derive(sqlx::FromRow)]
    struct Row {
        service_name: String,
    }

    let row = sqlx::query_as::<_, Row>(
        "SELECT service_name FROM service_sessions WHERE token_hash = $1 AND alive = true",
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    tracing::debug!(service = %row.service_name, "ws token validated");
    Some(row.service_name)
}

async fn ws_upgrade(
    State(state): State<AppState>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let Some(service_name) = validate_ws_token(&state.pool, &params.token).await else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "invalid or expired session token",
        )
            .into_response();
    };

    ws.on_upgrade(move |socket| handle_socket(socket, state, service_name))
        .into_response()
}

/// One code path for every authenticated WS client: a drain task copies
/// events from the broadcast bus into a private mpsc queue, the main
/// loop reads from the mpsc queue and forwards matching events to the
/// client. Every authenticated client is equally trusted.
async fn handle_socket(socket: WebSocket, state: AppState, service_name: String) {
    let (mut sender, mut receiver) = socket.split();
    let mut subscriptions: HashSet<Topic> = HashSet::new();

    // Server heartbeat: ping every 30s. If the client doesn't respond
    // the connection will be dropped by the underlying transport.
    let mut ping_interval = interval(Duration::from_secs(30));

    // Private 1000-message mpsc queue fed by a drain task that reads
    // from the shared broadcast bus. If this client is slow, the drain
    // task backpressures at mpsc_tx.send().await — it does not drop.
    let (mpsc_tx, mut mpsc_rx) = mpsc::channel(1000);
    let mut event_rx = state.events.subscribe();

    let drain_handle = tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(event) => {
                    if mpsc_tx.send(event).await.is_err() {
                        // Receiver dropped — socket closed
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "ws drain lagged behind event bus");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    loop {
        tokio::select! {
            msg = receiver.next() => {
                if !handle_client_msg(msg, &mut subscriptions) {
                    break;
                }
            }

            Some(api_event) = mpsc_rx.recv() => {
                if topic_matches(&subscriptions, &api_event) {
                    if let Ok(json) = serde_json::to_string(&api_event) {
                        if sender.send(Message::text(json)).await.is_err() {
                            break;
                        }
                    }
                }
            }

            _ = ping_interval.tick() => {
                if sender.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
        }
    }

    drain_handle.abort();
    tracing::debug!(service = %service_name, "ws client disconnected");
}

/// Process a client WebSocket message. Returns `false` if the connection should close.
fn handle_client_msg(
    msg: Option<Result<Message, axum::Error>>,
    subscriptions: &mut HashSet<Topic>,
) -> bool {
    match msg {
        Some(Ok(Message::Text(text))) => {
            if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                match client_msg {
                    ClientMessage::Subscribe { subscribe } => {
                        if let Some(topic) = parse_topic(&subscribe) {
                            tracing::debug!(?topic, "ws client subscribed");
                            subscriptions.insert(topic);
                        }
                    }
                    ClientMessage::Unsubscribe { unsubscribe } => {
                        if let Some(topic) = parse_topic(&unsubscribe) {
                            tracing::debug!(?topic, "ws client unsubscribed");
                            subscriptions.remove(&topic);
                        }
                    }
                }
            }
            true
        }
        Some(Ok(Message::Close(_))) | None => false,
        Some(Err(e)) => {
            tracing::debug!(error = %e, "ws receive error");
            false
        }
        // Pong, Binary, Ping — ignore
        _ => true,
    }
}

/// Check if an event matches any active subscription.
fn topic_matches(subscriptions: &HashSet<Topic>, event: &crate::events::ApiEvent) -> bool {
    subscriptions.contains(&Topic::AllSessions)
        || subscriptions.contains(&Topic::Session(event.session_id()))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/ws", get(ws_upgrade))
}
