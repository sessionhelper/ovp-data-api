use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::middleware::ServiceSession;
use crate::db::sessions as db;
use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub user_pseudo_id: Option<String>,
}

async fn create_session(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Result<Json<db::Session>, AppError> {
    let svc = request.extensions().get::<ServiceSession>()
        .ok_or_else(|| AppError::Internal("missing service session".to_string()))?;
    tracing::info!(service = %svc.service_name, "create session");

    let body = axum::body::to_bytes(request.into_body(), 1024 * 64)
        .await
        .map_err(|e| AppError::BadRequest(format!("read body: {e}")))?;
    let input: db::CreateSession = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("invalid json: {e}")))?;

    let session = db::create(&state.pool, &input).await?;
    Ok(Json(session))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    request: axum::extract::Request,
) -> Result<Json<db::Session>, AppError> {
    let svc = request.extensions().get::<ServiceSession>()
        .ok_or_else(|| AppError::Internal("missing service session".to_string()))?;
    tracing::debug!(service = %svc.service_name, session_id = %id, "get session");

    let session = db::get(&state.pool, id).await?;
    Ok(Json(session))
}

async fn update_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<db::UpdateSession>,
) -> Result<Json<db::Session>, AppError> {
    let session = db::update(&state.pool, id, &input).await?;
    Ok(Json(session))
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<db::Session>>, AppError> {
    let pseudo_id = query.user_pseudo_id
        .ok_or_else(|| AppError::BadRequest("user_pseudo_id query param required".to_string()))?;

    let sessions = db::list_by_user(&state.pool, &pseudo_id).await?;
    Ok(Json(sessions))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/sessions", post(create_session).get(list_sessions))
        .route("/internal/sessions/{id}", get(get_session).patch(update_session))
}
