//! Mute ranges — post-hoc time overlays that clients merge at render time.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::ids::PseudoId;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MuteRange {
    pub id: Uuid,
    pub session_id: Uuid,
    pub pseudo_id: PseudoId,
    pub start_offset_ms: i64,
    pub end_offset_ms: i64,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMuteRange {
    pub start_offset_ms: i64,
    pub end_offset_ms: i64,
    pub reason: Option<String>,
}

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    pseudo_id: &PseudoId,
    input: &CreateMuteRange,
) -> Result<MuteRange, AppError> {
    if input.end_offset_ms < input.start_offset_ms {
        return Err(AppError::BadRequest(
            "end_offset_ms < start_offset_ms".to_string(),
        ));
    }
    let row = sqlx::query_as::<_, MuteRange>(
        "INSERT INTO mute_ranges (session_id, pseudo_id, start_offset_ms, end_offset_ms, reason)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING *",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .bind(input.start_offset_ms)
    .bind(input.end_offset_ms)
    .bind(&input.reason)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(
    pool: &PgPool,
    session_id: Uuid,
    pseudo_id: &PseudoId,
) -> Result<Vec<MuteRange>, AppError> {
    let rows = sqlx::query_as::<_, MuteRange>(
        "SELECT * FROM mute_ranges WHERE session_id = $1 AND pseudo_id = $2
         ORDER BY start_offset_ms ASC",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_one(
    pool: &PgPool,
    session_id: Uuid,
    pseudo_id: &PseudoId,
    range_id: Uuid,
) -> Result<MuteRange, AppError> {
    let row = sqlx::query_as::<_, MuteRange>(
        "DELETE FROM mute_ranges
         WHERE id = $1 AND session_id = $2 AND pseudo_id = $3
         RETURNING *",
    )
    .bind(range_id)
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("mute range {range_id} not found")))?;
    Ok(row)
}

/// Delete all mute rows under a session (for the session-level cascade).
pub async fn delete_for_session(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
) -> Result<u64, AppError> {
    let res = sqlx::query("DELETE FROM mute_ranges WHERE session_id = $1")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    Ok(res.rows_affected())
}

pub async fn delete_for_participant(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    pseudo_id: &PseudoId,
) -> Result<u64, AppError> {
    let res = sqlx::query(
        "DELETE FROM mute_ranges WHERE session_id = $1 AND pseudo_id = $2",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected())
}
