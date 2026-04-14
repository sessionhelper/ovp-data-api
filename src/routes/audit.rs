//! Audit-log query endpoint.

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};

use crate::db::audit_log;
use crate::error::AppError;
use crate::routes::AppState;

async fn query(
    State(state): State<AppState>,
    Query(filter): Query<audit_log::QueryFilter>,
) -> Result<Json<Vec<audit_log::AuditRow>>, AppError> {
    Ok(Json(audit_log::query(&state.pool, &filter).await?))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/internal/audit", get(query))
}
