//! Session lifecycle routes.
//!
//! The state machine lives in [`crate::state`]; this module plumbs it to
//! HTTP: PATCH `/internal/sessions/{id}` proposes a transition, POST
//! `/internal/sessions/{id}/resume` handles the TTL'd abandoned → recording
//! edge, and POST `/internal/sessions/{id}/delete` cascades.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::ServiceSession;
use crate::db::{audit_log, chunks, metadata, mute_ranges, sessions, uniform};
use crate::error::AppError;
use crate::events::{Event, SessionStateChanged};
use crate::routes::AppState;
use crate::state::{can_transition, SessionStatus};

async fn create(
    State(state): State<AppState>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<sessions::CreateSession>,
) -> Result<(StatusCode, Json<sessions::Session>), AppError> {
    let session = sessions::create(&state.pool, &input).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(session.id),
            resource_type: "session",
            resource_id: session.id.to_string(),
            action: "created",
            detail: Some(json!({"status": session.status.as_str()})),
        },
    )
    .await?;
    state.metrics.inc_counter("chronicle_sessions_created_total", 1);
    Ok((StatusCode::CREATED, Json(session)))
}

async fn fetch(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<sessions::Session>, AppError> {
    Ok(Json(sessions::get(&state.pool, id).await?))
}

async fn list(
    State(state): State<AppState>,
    Query(filter): Query<sessions::ListFilter>,
) -> Result<Json<Vec<sessions::Session>>, AppError> {
    Ok(Json(sessions::list(&state.pool, &filter).await?))
}

#[derive(Debug, Deserialize)]
pub struct PatchSession {
    pub status: Option<SessionStatus>,
    #[serde(default, deserialize_with = "deserialize_optional_optional")]
    pub ended_at: Option<Option<chrono::DateTime<Utc>>>,
    #[serde(default, deserialize_with = "deserialize_optional_optional")]
    pub participant_count: Option<Option<i32>>,
}

/// Allow the client to distinguish `{"key": null}` (nullify) from `{}`
/// (leave unchanged). Not yet needed, but the PATCH contract reserves the
/// shape for future use; a single-`Option` serde default would conflate them.
fn deserialize_optional_optional<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    let v = Option::<T>::deserialize(d)?;
    Ok(Some(v))
}

async fn patch(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
    Json(body): Json<PatchSession>,
) -> Result<Json<sessions::Session>, AppError> {
    let mut tx = state.pool.begin().await?;
    let existing = sessions::get_in_tx(&mut tx, id).await?;

    let mut updated = existing.clone();
    let mut emit_event: Option<(SessionStatus, SessionStatus)> = None;

    if let Some(new_status) = body.status {
        if !can_transition(existing.status, new_status) {
            return Err(AppError::IllegalTransition {
                from: existing.status.to_string(),
                to: new_status.to_string(),
            });
        }
        updated = sessions::update_status(&mut tx, id, new_status).await?;
        if existing.status != new_status {
            emit_event = Some((existing.status, new_status));
        }
    }

    if body.ended_at.is_some() || body.participant_count.is_some() {
        updated = sessions::patch_fields(&mut tx, id, body.ended_at, body.participant_count).await?;
    }

    audit_log::append_tx(
        &mut tx,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(id),
            resource_type: "session",
            resource_id: id.to_string(),
            action: "patched",
            detail: Some(json!({
                "status": updated.status.as_str(),
                "ended_at": updated.ended_at,
            })),
        },
    )
    .await?;
    tx.commit().await?;

    if let Some((old, new)) = emit_event {
        let _ = state.events.send(Event::SessionStateChanged {
            at_ts: Utc::now(),
            data: SessionStateChanged {
                session_id: id,
                guild_id: updated.guild_id,
                old,
                new,
            },
        });
        state.metrics.inc_counter(
            &format!("chronicle_session_state_{}_total", new.as_str()),
            1,
        );
    }
    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
pub struct ResumeBody {
    pub resumed_by_service_name: String,
    pub reason: Option<String>,
}

async fn resume(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
    Json(body): Json<ResumeBody>,
) -> Result<Json<sessions::Session>, AppError> {
    let mut tx = state.pool.begin().await?;
    let existing = sessions::get_in_tx(&mut tx, id).await?;

    if existing.status != SessionStatus::Abandoned {
        return Err(AppError::IllegalTransition {
            from: existing.status.to_string(),
            to: SessionStatus::Recording.to_string(),
        });
    }

    // TTL check — use abandoned_at if present, otherwise started_at as a
    // conservative fallback (shouldn't happen in practice because the state
    // transition stamp is set on entry).
    let anchor = existing.abandoned_at.unwrap_or(existing.started_at);
    let now = Utc::now();
    let age = (now - anchor).num_seconds().max(0) as u64;
    if age > state.resume_ttl.as_secs() {
        return Err(AppError::Gone(format!(
            "resume window expired: abandoned {age}s ago, limit {}s",
            state.resume_ttl.as_secs()
        )));
    }

    let updated = sessions::resume_from_abandoned(&mut tx, id).await?;
    audit_log::append_tx(
        &mut tx,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(id),
            resource_type: "session",
            resource_id: id.to_string(),
            action: "resumed",
            detail: Some(json!({
                "by": body.resumed_by_service_name,
                "reason": body.reason,
            })),
        },
    )
    .await?;
    tx.commit().await?;

    let _ = state.events.send(Event::SessionStateChanged {
        at_ts: Utc::now(),
        data: SessionStateChanged {
            session_id: id,
            guild_id: updated.guild_id,
            old: SessionStatus::Abandoned,
            new: SessionStatus::Recording,
        },
    });
    Ok(Json(updated))
}

async fn delete_cascade(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Collect chunk S3 keys + wipe all dependent rows inside a single tx.
    // S3 deletes happen after commit, on best-effort basis.
    let mut tx = state.pool.begin().await?;
    let existing = sessions::get_in_tx(&mut tx, id).await?;

    let keys = chunks::delete_for_session(&mut tx, id).await?;
    let _segs = uniform::delete_all_for_session(&mut tx, uniform::SEGMENTS, id).await?;
    let _beats = uniform::delete_all_for_session(&mut tx, uniform::BEATS, id).await?;
    let _scenes = uniform::delete_all_for_session(&mut tx, uniform::SCENES, id).await?;
    let _mutes = mute_ranges::delete_for_session(&mut tx, id).await?;
    // metadata, participants cascade via ON DELETE on session_id FK; we
    // explicitly null the metadata row to make intent visible.
    sqlx::query("DELETE FROM session_metadata WHERE session_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM session_participants WHERE session_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    let updated = sessions::update_status(&mut tx, id, SessionStatus::Deleted).await?;
    audit_log::append_tx(
        &mut tx,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(id),
            resource_type: "session",
            resource_id: id.to_string(),
            action: "deleted",
            detail: Some(json!({ "chunk_count": keys.len() })),
        },
    )
    .await?;
    tx.commit().await?;

    // Best-effort S3 sweep. Failures here are logged but don't fail the API
    // response — the DB is the source of truth and the tombstone is set.
    if !keys.is_empty() {
        if let Err(e) = state.store.delete_many(&keys).await {
            tracing::warn!(error = %e, count = keys.len(), "s3 cascade delete partial");
        }
    }

    let _ = state.events.send(Event::SessionStateChanged {
        at_ts: Utc::now(),
        data: SessionStateChanged {
            session_id: id,
            guild_id: existing.guild_id,
            old: existing.status,
            new: SessionStatus::Deleted,
        },
    });
    Ok(Json(json!({
        "ok": true,
        "tombstone_status": updated.status.as_str(),
        "s3_keys_deleted": keys.len(),
    })))
}

async fn summary(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<sessions::SessionSummary>, AppError> {
    Ok(Json(sessions::summary(&state.pool, id).await?))
}

// --- small helper to make sure the handler ignores a missing blob --
#[allow(dead_code)]
async fn _metadata_is_present(pool: &sqlx::PgPool, id: Uuid) -> Result<bool, AppError> {
    Ok(metadata::get_opt(pool, id).await?.is_some())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/sessions", post(create).get(list))
        .route("/internal/sessions/{id}", get(fetch).patch(patch))
        .route("/internal/sessions/{id}/resume", post(resume))
        .route("/internal/sessions/{id}/delete", post(delete_cascade))
        .route("/internal/sessions/{id}/summary", get(summary))
}
