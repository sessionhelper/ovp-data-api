//! Admin grant backchannel.
//!
//! Authenticated with the shared secret only — the whole point of this
//! endpoint is to bootstrap the first admin before any service-session
//! handoff has happened.

use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::db::{audit_log, users};
use crate::error::AppError;
use crate::ids::PseudoId;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct GrantRequest {
    pub shared_secret: String,
    pub pseudo_id: PseudoId,
    #[serde(default = "default_true")]
    pub is_admin: bool,
}

fn default_true() -> bool {
    true
}

async fn grant(
    State(state): State<AppState>,
    Json(req): Json<GrantRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if req.shared_secret != state.shared_secret {
        return Err(AppError::Unauthorized("invalid shared secret".to_string()));
    }

    let mut tx = state.pool.begin().await?;
    let user = users::set_admin(&mut tx, &req.pseudo_id, req.is_admin).await?;
    audit_log::append_tx(
        &mut tx,
        &audit_log::Entry {
            actor_service: "admin_bootstrap",
            actor_pseudo: Some(req.pseudo_id.as_str()),
            session_id: None,
            resource_type: "admin",
            resource_id: req.pseudo_id.as_str().to_string(),
            action: if req.is_admin { "granted" } else { "revoked" },
            detail: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "ok": true,
        "pseudo_id": user.pseudo_id,
        "is_admin": user.is_admin,
    })))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/internal/admin/grant", post(grant))
}
