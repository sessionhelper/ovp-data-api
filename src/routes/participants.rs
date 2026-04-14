//! Participant management endpoints.

use axum::{
    extract::{Path, State},
    routing::{get, patch, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::ServiceSession;
use crate::db::{audit_log, participants};
use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct Batch {
    pub participants: Vec<participants::AddParticipant>,
}

async fn add_one(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<participants::AddParticipant>,
) -> Result<Json<participants::Participant>, AppError> {
    let row = participants::add(&state.pool, session_id, &input).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(row.pseudo_id.as_str()),
            session_id: Some(session_id),
            resource_type: "participant",
            resource_id: row.id.to_string(),
            action: "added",
            detail: None,
        },
    )
    .await?;
    Ok(Json(row))
}

async fn add_batch(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<Batch>,
) -> Result<Json<Vec<participants::Participant>>, AppError> {
    let rows = participants::add_many(&state.pool, session_id, &input.participants).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(session_id),
            resource_type: "participant",
            resource_id: session_id.to_string(),
            action: "batch_added",
            detail: Some(json!({"count": rows.len()})),
        },
    )
    .await?;
    Ok(Json(rows))
}

async fn list(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<participants::Participant>>, AppError> {
    Ok(Json(participants::list(&state.pool, session_id).await?))
}

async fn fetch(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<participants::Participant>, AppError> {
    Ok(Json(participants::get(&state.pool, id).await?))
}

async fn patch_consent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<participants::UpdateConsent>,
) -> Result<Json<participants::Participant>, AppError> {
    let row = participants::update_consent(&state.pool, id, &input).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(row.pseudo_id.as_str()),
            session_id: Some(row.session_id),
            resource_type: "participant",
            resource_id: id.to_string(),
            action: "consent_patched",
            detail: Some(json!({"consent_scope": row.consent_scope})),
        },
    )
    .await?;
    Ok(Json(row))
}

async fn patch_license(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<participants::UpdateLicense>,
) -> Result<Json<participants::Participant>, AppError> {
    let row = participants::update_license(&state.pool, id, &input).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(row.pseudo_id.as_str()),
            session_id: Some(row.session_id),
            resource_type: "participant",
            resource_id: id.to_string(),
            action: "license_patched",
            detail: Some(json!({
                "no_llm_training": row.no_llm_training,
                "no_public_release": row.no_public_release,
            })),
        },
    )
    .await?;
    Ok(Json(row))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/internal/sessions/{id}/participants",
            post(add_one).get(list),
        )
        .route("/internal/sessions/{id}/participants/batch", post(add_batch))
        .route("/internal/participants/{id}", get(fetch))
        .route("/internal/participants/{id}/consent", patch(patch_consent))
        .route("/internal/participants/{id}/license", patch(patch_license))
}
