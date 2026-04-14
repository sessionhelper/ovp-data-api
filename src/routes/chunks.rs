//! PCM chunk upload + download.
//!
//! Upload is idempotent on `X-Client-Chunk-Id`: a retry returns the
//! existing row's `{seq, s3_key}` without re-emitting the WS event or
//! duplicating the S3 object.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::ServiceSession;
use crate::db::{audit_log, chunks, sessions};
use crate::error::AppError;
use crate::events::{ChunkUploaded, Event};
use crate::ids::PseudoId;
use crate::routes::AppState;

/// Hard max chunk body (spec §5): 3 MB.
pub const MAX_CHUNK_BYTES: usize = 3 * 1024 * 1024;

#[derive(Debug, Serialize)]
struct UploadResponse {
    seq: i32,
    s3_key: String,
    deduplicated: bool,
}

struct ChunkHeaders {
    capture_started_at: DateTime<Utc>,
    duration_ms: i32,
    client_chunk_id: String,
}

fn parse_chunk_headers(h: &HeaderMap) -> Result<ChunkHeaders, AppError> {
    let capture = h
        .get("x-capture-started-at")
        .ok_or(AppError::MissingHeader("X-Capture-Started-At"))?
        .to_str()
        .map_err(|_| AppError::BadRequest("X-Capture-Started-At: non-ASCII".into()))?;
    let capture_started_at = DateTime::parse_from_rfc3339(capture)
        .map_err(|e| AppError::BadRequest(format!("X-Capture-Started-At: {e}")))?
        .with_timezone(&Utc);

    let duration_raw = h
        .get("x-duration-ms")
        .ok_or(AppError::MissingHeader("X-Duration-Ms"))?
        .to_str()
        .map_err(|_| AppError::BadRequest("X-Duration-Ms: non-ASCII".into()))?;
    let duration_ms = duration_raw
        .parse::<i32>()
        .map_err(|e| AppError::BadRequest(format!("X-Duration-Ms: {e}")))?;
    if duration_ms < 0 {
        return Err(AppError::BadRequest("X-Duration-Ms: must be >= 0".into()));
    }

    let client_chunk_id = h
        .get("x-client-chunk-id")
        .ok_or(AppError::MissingHeader("X-Client-Chunk-Id"))?
        .to_str()
        .map_err(|_| AppError::BadRequest("X-Client-Chunk-Id: non-ASCII".into()))?
        .to_string();
    if client_chunk_id.is_empty() {
        return Err(AppError::BadRequest(
            "X-Client-Chunk-Id: must be non-empty".into(),
        ));
    }

    Ok(ChunkHeaders {
        capture_started_at,
        duration_ms,
        client_chunk_id,
    })
}

async fn upload(
    State(state): State<AppState>,
    Path((session_id, pseudo_raw)): Path<(Uuid, String)>,
    Extension(svc): Extension<ServiceSession>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<UploadResponse>, AppError> {
    let pseudo_id = PseudoId::new_or_mixed(pseudo_raw)?;
    let h = parse_chunk_headers(&headers)?;

    // Idempotency lookup first — return early without touching S3 or the
    // event bus on retry.
    let mut tx = state.pool.begin().await?;
    if let Some(existing) =
        chunks::lookup_by_client_id(&mut tx, session_id, &pseudo_id, &h.client_chunk_id).await?
    {
        tx.rollback().await?;
        return Ok(Json(UploadResponse {
            seq: existing.seq,
            s3_key: existing.s3_key,
            deduplicated: true,
        }));
    }

    // Session must exist and have an s3_prefix.
    let session = sessions::get_in_tx(&mut tx, session_id).await?;

    // Read body with the hard cap.
    let bytes = axum::body::to_bytes(body, MAX_CHUNK_BYTES + 1)
        .await
        .map_err(|e| AppError::BadRequest(format!("read body: {e}")))?;
    if bytes.len() > MAX_CHUNK_BYTES {
        return Err(AppError::PayloadTooLarge(format!(
            "chunk exceeds {MAX_CHUNK_BYTES} bytes"
        )));
    }
    let size_bytes = bytes.len() as i32;

    // Reserve a DB row (and with it a seq value) first, so that a successful
    // S3 put can never "win" over a racing client. If the subsequent put fails
    // we delete the row and return 502 — the client retries with the same
    // chunk_id and gets a fresh seq.
    let row = chunks::insert(
        &mut tx,
        session_id,
        &pseudo_id,
        "", // placeholder s3_key, we patch after upload
        size_bytes,
        h.capture_started_at,
        h.duration_ms,
        &h.client_chunk_id,
    )
    .await?;

    let s3_key = format!(
        "{}/audio/{}/chunk_{:06}.pcm",
        session.s3_prefix, pseudo_id, row.seq
    );

    // Now actually upload to S3. Commit only if both succeed.
    let t0 = std::time::Instant::now();
    state
        .store
        .put(&s3_key, bytes.to_vec(), "audio/pcm")
        .await?;
    state
        .metrics
        .observe_histogram("chronicle_s3_put_seconds", t0.elapsed().as_secs_f64());
    state
        .metrics
        .inc_counter("chronicle_chunks_uploaded_total", 1);

    sqlx::query("UPDATE session_chunks SET s3_key = $1 WHERE session_id = $2 AND pseudo_id = $3 AND seq = $4")
        .bind(&s3_key)
        .bind(session_id)
        .bind(pseudo_id.as_str())
        .bind(row.seq)
        .execute(&mut *tx)
        .await?;

    audit_log::append_tx(
        &mut tx,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(session_id),
            resource_type: "chunk",
            resource_id: format!("{session_id}/{pseudo_id}/{seq}", seq = row.seq),
            action: "uploaded",
            detail: Some(json!({"size_bytes": size_bytes})),
        },
    )
    .await?;
    tx.commit().await?;

    let _ = state.events.send(Event::ChunkUploaded {
        at_ts: Utc::now(),
        data: ChunkUploaded {
            session_id,
            guild_id: session.guild_id,
            pseudo_id,
            seq: row.seq,
            size_bytes: size_bytes as i64,
        },
    });

    Ok(Json(UploadResponse {
        seq: row.seq,
        s3_key,
        deduplicated: false,
    }))
}

async fn list(
    State(state): State<AppState>,
    Path((session_id, pseudo_raw)): Path<(Uuid, String)>,
) -> Result<Json<Vec<chunks::ChunkRow>>, AppError> {
    let pseudo_id = PseudoId::new_or_mixed(pseudo_raw)?;
    Ok(Json(chunks::list(&state.pool, session_id, &pseudo_id).await?))
}

async fn download(
    State(state): State<AppState>,
    Path((session_id, pseudo_raw, seq)): Path<(Uuid, String, i32)>,
) -> Result<impl IntoResponse, AppError> {
    let pseudo_id = PseudoId::new_or_mixed(pseudo_raw)?;
    let row = chunks::get(&state.pool, session_id, &pseudo_id, seq).await?;
    let t0 = std::time::Instant::now();
    let bytes = state.store.get(&row.s3_key).await?;
    state
        .metrics
        .observe_histogram("chronicle_s3_get_seconds", t0.elapsed().as_secs_f64());

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "audio/pcm")],
        bytes,
    ))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/internal/sessions/{id}/audio/{pseudo_id}/chunk",
            post(upload),
        )
        .route(
            "/internal/sessions/{id}/audio/{pseudo_id}/chunks",
            get(list),
        )
        .route(
            "/internal/sessions/{id}/audio/{pseudo_id}/chunk/{seq}",
            get(download),
        )
}
