//! Sessions table access.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::SessionStatus;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: Uuid,
    pub guild_id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub abandoned_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub game_system: Option<String>,
    pub campaign_name: Option<String>,
    pub participant_count: Option<i32>,
    pub s3_prefix: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSession {
    pub id: Uuid,
    pub guild_id: i64,
    pub started_at: DateTime<Utc>,
    pub game_system: Option<String>,
    pub campaign_name: Option<String>,
    pub s3_prefix: String,
}

pub async fn create(pool: &PgPool, input: &CreateSession) -> Result<Session, AppError> {
    let row = sqlx::query_as::<_, Session>(
        "INSERT INTO sessions (id, guild_id, started_at, game_system, campaign_name, s3_prefix, status)
         VALUES ($1, $2, $3, $4, $5, $6, 'recording')
         RETURNING *",
    )
    .bind(input.id)
    .bind(input.guild_id)
    .bind(input.started_at)
    .bind(&input.game_system)
    .bind(&input.campaign_name)
    .bind(&input.s3_prefix)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Session, AppError> {
    sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session {id} not found")))
}

pub async fn get_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<Session, AppError> {
    sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session {id} not found")))
}

#[derive(Debug, Deserialize)]
pub struct ListFilter {
    pub status: Option<String>,
    pub guild_id: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list(pool: &PgPool, f: &ListFilter) -> Result<Vec<Session>, AppError> {
    // Build a parameterized query with optional filters. The indexes on
    // (status) and (guild_id, status) keep each filter combination cheap.
    let status_parsed = match &f.status {
        Some(s) => Some(s.parse::<SessionStatus>()?),
        None => None,
    };

    let rows = sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions
         WHERE ($1::text IS NULL OR status = $1::text)
           AND ($2::bigint IS NULL OR guild_id = $2::bigint)
         ORDER BY started_at DESC
         LIMIT COALESCE($3, 100)
         OFFSET COALESCE($4, 0)",
    )
    .bind(status_parsed.map(|s| s.as_str().to_string()))
    .bind(f.guild_id)
    .bind(f.limit)
    .bind(f.offset)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Apply a status transition against the DB without verifying legality.
/// The route layer is responsible for checking `state::can_transition`
/// first; this function only writes.
pub async fn update_status(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    status: SessionStatus,
) -> Result<Session, AppError> {
    let row = sqlx::query_as::<_, Session>(
        "UPDATE sessions
         SET status = $2,
             abandoned_at = CASE WHEN $2 = 'abandoned' THEN NOW() ELSE abandoned_at END,
             deleted_at   = CASE WHEN $2 = 'deleted'   THEN NOW() ELSE deleted_at END
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(status.as_str())
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

/// Patch non-status session fields.
pub async fn patch_fields(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    ended_at: Option<Option<DateTime<Utc>>>,
    participant_count: Option<Option<i32>>,
) -> Result<Session, AppError> {
    let row = sqlx::query_as::<_, Session>(
        "UPDATE sessions SET
            ended_at = CASE WHEN $2::bool THEN $3::timestamptz ELSE ended_at END,
            participant_count = CASE WHEN $4::bool THEN $5::int ELSE participant_count END
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(ended_at.is_some())
    .bind(ended_at.flatten())
    .bind(participant_count.is_some())
    .bind(participant_count.flatten())
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

pub async fn resume_from_abandoned(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<Session, AppError> {
    let row = sqlx::query_as::<_, Session>(
        "UPDATE sessions
         SET status = 'recording', abandoned_at = NULL
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub session_id: Uuid,
    pub chunk_count: i64,
    pub participant_count: i64,
    pub duration_ms: i64,
    pub segment_count: i64,
    pub beat_count: i64,
    pub scene_count: i64,
    pub mute_range_count: i64,
    pub aggregate_license_flags: LicenseFlags,
}

#[derive(Debug, Clone, Serialize)]
pub struct LicenseFlags {
    pub no_llm_training: bool,
    pub no_public_release: bool,
}

pub async fn summary(pool: &PgPool, id: Uuid) -> Result<SessionSummary, AppError> {
    // Single round-trip: one SELECT with scalar subqueries.
    #[derive(sqlx::FromRow)]
    struct Row {
        chunk_count: i64,
        participant_count: i64,
        duration_ms: i64,
        segment_count: i64,
        beat_count: i64,
        scene_count: i64,
        mute_range_count: i64,
        any_no_llm: bool,
        any_no_public: bool,
    }

    let row = sqlx::query_as::<_, Row>(
        "SELECT
            COALESCE((SELECT SUM(duration_ms)::bigint FROM session_chunks WHERE session_id = $1), 0)                AS duration_ms,
            (SELECT COUNT(*)::bigint FROM session_chunks     WHERE session_id = $1)                                 AS chunk_count,
            (SELECT COUNT(*)::bigint FROM session_participants WHERE session_id = $1)                               AS participant_count,
            (SELECT COUNT(*)::bigint FROM session_segments    WHERE session_id = $1)                                AS segment_count,
            (SELECT COUNT(*)::bigint FROM session_beats       WHERE session_id = $1)                                AS beat_count,
            (SELECT COUNT(*)::bigint FROM session_scenes      WHERE session_id = $1)                                AS scene_count,
            (SELECT COUNT(*)::bigint FROM mute_ranges         WHERE session_id = $1)                                AS mute_range_count,
            COALESCE((SELECT bool_or(no_llm_training)  FROM session_participants WHERE session_id = $1), false)     AS any_no_llm,
            COALESCE((SELECT bool_or(no_public_release) FROM session_participants WHERE session_id = $1), false)    AS any_no_public
        ",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(SessionSummary {
        session_id: id,
        chunk_count: row.chunk_count,
        participant_count: row.participant_count,
        duration_ms: row.duration_ms,
        segment_count: row.segment_count,
        beat_count: row.beat_count,
        scene_count: row.scene_count,
        mute_range_count: row.mute_range_count,
        aggregate_license_flags: LicenseFlags {
            no_llm_training: row.any_no_llm,
            no_public_release: row.any_no_public,
        },
    })
}
