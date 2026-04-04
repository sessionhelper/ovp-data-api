use axum::{
    extract::{Query, State},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::db::audit as db;
use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub user_id: Option<Uuid>,
}

async fn create_entry(
    State(state): State<AppState>,
    Json(input): Json<db::CreateAuditEntry>,
) -> Result<Json<db::AuditEntry>, AppError> {
    let entry = db::create(&state.pool, &input).await?;
    Ok(Json(entry))
}

async fn list_entries(
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<db::AuditEntry>>, AppError> {
    let user_id = query.user_id
        .ok_or_else(|| AppError::BadRequest("user_id query param required".to_string()))?;

    let entries = db::list_by_user(&state.pool, user_id).await?;
    Ok(Json(entries))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/audit", post(create_entry).get(list_entries))
}
