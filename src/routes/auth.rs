//! `POST /internal/auth` + `POST /internal/heartbeat`.

use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::auth::{generate_token, hash_token};
use crate::db::service_sessions;
use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub shared_secret: String,
    pub service_name: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub session_token: String,
}

async fn authenticate(
    State(state): State<AppState>,
    Json(req): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    if req.shared_secret != state.shared_secret {
        return Err(AppError::Unauthorized("invalid shared secret".to_string()));
    }
    let token = generate_token();
    let hash = hash_token(&token);
    service_sessions::insert(&state.pool, &req.service_name, &hash).await?;
    info!(service = %req.service_name, "service session created");
    Ok(Json(AuthResponse {
        session_token: token,
    }))
}

async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("missing bearer token".to_string()))?;
    let hash = hash_token(token);
    let ok = service_sessions::touch(&state.pool, &hash).await?;
    if !ok {
        return Err(AppError::Unauthorized("invalid session token".to_string()));
    }
    Ok(Json(serde_json::json!({"ok": true})))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/auth", post(authenticate))
        .route("/internal/heartbeat", post(heartbeat))
}
