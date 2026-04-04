use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Participant {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Option<Uuid>,
    pub consent_scope: Option<String>,
    pub consented_at: Option<DateTime<Utc>>,
    pub withdrawn_at: Option<DateTime<Utc>>,
    pub mid_session_join: bool,
    pub no_llm_training: bool,
    pub no_public_release: bool,
}

#[derive(Debug, Deserialize)]
pub struct AddParticipant {
    pub user_id: Option<Uuid>,
    pub mid_session_join: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateConsent {
    pub consent_scope: Option<String>,
    pub consented_at: Option<DateTime<Utc>>,
    pub withdrawn_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLicense {
    pub no_llm_training: Option<bool>,
    pub no_public_release: Option<bool>,
}

pub async fn add(pool: &PgPool, session_id: Uuid, input: &AddParticipant) -> Result<Participant, AppError> {
    let row = sqlx::query_as::<_, Participant>(
        "INSERT INTO session_participants (session_id, user_id, mid_session_join)
         VALUES ($1, $2, COALESCE($3, false))
         RETURNING *"
    )
    .bind(session_id)
    .bind(input.user_id)
    .bind(input.mid_session_join)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

pub async fn list(pool: &PgPool, session_id: Uuid) -> Result<Vec<Participant>, AppError> {
    let rows = sqlx::query_as::<_, Participant>(
        "SELECT * FROM session_participants WHERE session_id = $1"
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn update_consent(pool: &PgPool, id: Uuid, input: &UpdateConsent) -> Result<Participant, AppError> {
    let row = sqlx::query_as::<_, Participant>(
        "UPDATE session_participants SET
            consent_scope = COALESCE($2, consent_scope),
            consented_at = COALESCE($3, consented_at),
            withdrawn_at = COALESCE($4, withdrawn_at)
         WHERE id = $1
         RETURNING *"
    )
    .bind(id)
    .bind(&input.consent_scope)
    .bind(input.consented_at)
    .bind(input.withdrawn_at)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("participant {id} not found")))?;

    Ok(row)
}

pub async fn update_license(pool: &PgPool, id: Uuid, input: &UpdateLicense) -> Result<Participant, AppError> {
    let row = sqlx::query_as::<_, Participant>(
        "UPDATE session_participants SET
            no_llm_training = COALESCE($2, no_llm_training),
            no_public_release = COALESCE($3, no_public_release)
         WHERE id = $1
         RETURNING *"
    )
    .bind(id)
    .bind(input.no_llm_training)
    .bind(input.no_public_release)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("participant {id} not found")))?;

    Ok(row)
}
