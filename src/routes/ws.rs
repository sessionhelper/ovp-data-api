//! WebSocket endpoint for real-time event streaming.
//!
//! Clients connect to `GET /ws`, then send JSON subscription messages
//! to choose which events they receive. The server filters the global
//! event bus by the client's active subscriptions and forwards matching
//! events as JSON text frames.
//!
//! # Subscription protocol
//!
//! ```json
//! {"subscribe": "sessions/<uuid>"}   // events for one session
//! {"subscribe": "sessions"}           // all session events
//! {"unsubscribe": "sessions/<uuid>"} // stop receiving
//! ```
//!
//! # Auth
//!
//! TODO: add token-based auth for WS connections. Currently open for
//! internal services only — not exposed to the public internet.

use std::collections::HashSet;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::time::{interval, Duration};
use uuid::Uuid;

use crate::routes::AppState;

/// Client-to-server control message.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ClientMessage {
    Subscribe { subscribe: String },
    Unsubscribe { unsubscribe: String },
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

async fn ws_upgrade(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut subscriptions: HashSet<Topic> = HashSet::new();
    let mut event_rx = state.events.subscribe();

    // Server heartbeat: ping every 30s. If the client doesn't respond
    // the connection will be dropped by the underlying transport.
    let mut ping_interval = interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            // Incoming message from the client
            msg = receiver.next() => {
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
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::debug!("ws client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "ws receive error");
                        break;
                    }
                    // Pong, Binary, Ping — ignore
                    _ => {}
                }
            }

            // Event from the broadcast bus
            event = event_rx.recv() => {
                match event {
                    Ok(api_event) => {
                        let matches = subscriptions.contains(&Topic::AllSessions)
                            || subscriptions.contains(&Topic::Session(api_event.session_id()));

                        if matches {
                            if let Ok(json) = serde_json::to_string(&api_event) {
                                if sender.send(Message::text(json)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "ws client lagged behind event bus");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }

            // Heartbeat ping
            _ = ping_interval.tick() => {
                if sender.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/ws", get(ws_upgrade))
}
