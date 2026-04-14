//! Session participants — pure pseudo_id world.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::ids::PseudoId;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Participant {
    pub id: Uuid,
    pub session_id: Uuid,
    pub pseudo_id: PseudoId,
    pub consent_scope: Option<String>,
    pub consented_at: Option<DateTime<Utc>>,
    pub mid_session_join: bool,
    pub no_llm_training: bool,
    pub no_public_release: bool,
    pub data_wiped_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddParticipant {
    pub pseudo_id: PseudoId,
    #[serde(default)]
    pub mid_session_join: bool,
}

pub async fn add(
    pool: &PgPool,
    session_id: Uuid,
    input: &AddParticipant,
) -> Result<Participant, AppError> {
    // Upsert user first so the FK holds. This is a single statement via CTE.
    let row = sqlx::query_as::<_, Participant>(
        "WITH u AS (
            INSERT INTO users (pseudo_id) VALUES ($2)
            ON CONFLICT (pseudo_id) DO UPDATE SET pseudo_id = EXCLUDED.pseudo_id
            RETURNING pseudo_id
         )
         INSERT INTO session_participants (session_id, pseudo_id, mid_session_join)
         VALUES ($1, (SELECT pseudo_id FROM u), $3)
         ON CONFLICT (session_id, pseudo_id) DO UPDATE SET
            mid_session_join = EXCLUDED.mid_session_join
         RETURNING *",
    )
    .bind(session_id)
    .bind(input.pseudo_id.as_str())
    .bind(input.mid_session_join)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn add_many(
    pool: &PgPool,
    session_id: Uuid,
    inputs: &[AddParticipant],
) -> Result<Vec<Participant>, AppError> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }

    let mut tx = pool.begin().await?;

    // Seed users in one pass so FKs hold.
    let pseudos: Vec<String> = inputs.iter().map(|p| p.pseudo_id.as_str().to_string()).collect();
    sqlx::query(
        "INSERT INTO users (pseudo_id)
         SELECT unnest($1::text[])
         ON CONFLICT (pseudo_id) DO NOTHING",
    )
    .bind(&pseudos)
    .execute(&mut *tx)
    .await?;

    let mids: Vec<bool> = inputs.iter().map(|p| p.mid_session_join).collect();

    let rows = sqlx::query_as::<_, Participant>(
        "INSERT INTO session_participants (session_id, pseudo_id, mid_session_join)
         SELECT $1, p, m
         FROM unnest($2::text[], $3::bool[]) AS t(p, m)
         ON CONFLICT (session_id, pseudo_id) DO UPDATE SET
            mid_session_join = EXCLUDED.mid_session_join
         RETURNING *",
    )
    .bind(session_id)
    .bind(&pseudos)
    .bind(&mids)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(rows)
}

pub async fn list(pool: &PgPool, session_id: Uuid) -> Result<Vec<Participant>, AppError> {
    let rows = sqlx::query_as::<_, Participant>(
        "SELECT * FROM session_participants WHERE session_id = $1
         ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Participant, AppError> {
    sqlx::query_as::<_, Participant>("SELECT * FROM session_participants WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("participant {id} not found")))
}

#[derive(Debug, Deserialize)]
pub struct UpdateConsent {
    pub consent_scope: String,
    pub consented_at: Option<DateTime<Utc>>,
}

pub async fn update_consent(
    pool: &PgPool,
    id: Uuid,
    input: &UpdateConsent,
) -> Result<Participant, AppError> {
    if !matches!(input.consent_scope.as_str(), "full" | "decline" | "timed_out") {
        return Err(AppError::BadRequest(format!(
            "invalid consent_scope: {}",
            input.consent_scope
        )));
    }
    let row = sqlx::query_as::<_, Participant>(
        "UPDATE session_participants
         SET consent_scope = $2, consented_at = COALESCE($3, NOW())
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(&input.consent_scope)
    .bind(input.consented_at)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("participant {id} not found")))?;
    Ok(row)
}

#[derive(Debug, Deserialize)]
pub struct UpdateLicense {
    pub no_llm_training: Option<bool>,
    pub no_public_release: Option<bool>,
}

pub async fn update_license(
    pool: &PgPool,
    id: Uuid,
    input: &UpdateLicense,
) -> Result<Participant, AppError> {
    let row = sqlx::query_as::<_, Participant>(
        "UPDATE session_participants SET
            no_llm_training    = COALESCE($2, no_llm_training),
            no_public_release  = COALESCE($3, no_public_release)
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(input.no_llm_training)
    .bind(input.no_public_release)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("participant {id} not found")))?;
    Ok(row)
}

/// Mark a participant as wiped. Intended for hard-delete cascades; called
/// after the chunk + segment cleanup has succeeded.
pub async fn mark_wiped(
    pool: &PgPool,
    session_id: Uuid,
    pseudo_id: &PseudoId,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE session_participants
         SET data_wiped_at = NOW()
         WHERE session_id = $1 AND pseudo_id = $2",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .execute(pool)
    .await?;
    Ok(())
}
