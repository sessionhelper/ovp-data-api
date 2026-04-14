//! Display-name aliases for users. Idempotent upsert keyed on
//! `(pseudo_id, display_name)`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::error::AppError;
use crate::ids::PseudoId;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DisplayName {
    pub pseudo_id: PseudoId,
    pub display_name: String,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub seen_count: i32,
    pub source: String,
}

#[derive(Debug, Deserialize)]
pub struct UpsertDisplayName {
    pub display_name: String,
    pub source: String,
}

/// Idempotent upsert: increments `seen_count` and bumps `last_seen_at` on
/// conflict with the composite PK.
pub async fn upsert(
    pool: &PgPool,
    pseudo_id: &PseudoId,
    input: &UpsertDisplayName,
) -> Result<DisplayName, AppError> {
    if !matches!(input.source.as_str(), "bot" | "portal_override") {
        return Err(AppError::BadRequest(format!(
            "invalid source: {} (expected 'bot' or 'portal_override')",
            input.source
        )));
    }
    let row = sqlx::query_as::<_, DisplayName>(
        "INSERT INTO user_display_names (pseudo_id, display_name, source)
         VALUES ($1, $2, $3)
         ON CONFLICT (pseudo_id, display_name) DO UPDATE SET
            last_seen_at = NOW(),
            seen_count   = user_display_names.seen_count + 1
         RETURNING *",
    )
    .bind(pseudo_id.as_str())
    .bind(&input.display_name)
    .bind(&input.source)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, pseudo_id: &PseudoId) -> Result<Vec<DisplayName>, AppError> {
    let rows = sqlx::query_as::<_, DisplayName>(
        "SELECT * FROM user_display_names
         WHERE pseudo_id = $1
         ORDER BY last_seen_at DESC",
    )
    .bind(pseudo_id.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
