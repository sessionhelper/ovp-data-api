use axum::{
    extract::{Path, State},
    routing::{patch, post},
    Json, Router,
};
use uuid::Uuid;

use crate::db::participants as db;
use crate::error::AppError;
use crate::routes::AppState;

async fn add_participant(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(input): Json<db::AddParticipant>,
) -> Result<Json<db::Participant>, AppError> {
    let participant = db::add(&state.pool, session_id, &input).await?;
    Ok(Json(participant))
}

async fn list_participants(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<db::Participant>>, AppError> {
    let participants = db::list(&state.pool, session_id).await?;
    Ok(Json(participants))
}

async fn update_consent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<db::UpdateConsent>,
) -> Result<Json<db::Participant>, AppError> {
    let participant = db::update_consent(&state.pool, id, &input).await?;
    Ok(Json(participant))
}

async fn update_license(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<db::UpdateLicense>,
) -> Result<Json<db::Participant>, AppError> {
    let participant = db::update_license(&state.pool, id, &input).await?;
    Ok(Json(participant))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/sessions/{id}/participants", post(add_participant).get(list_participants))
        .route("/internal/participants/{id}/consent", patch(update_consent))
        .route("/internal/participants/{id}/license", patch(update_license))
}
