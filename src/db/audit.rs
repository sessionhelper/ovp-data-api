use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub action: String,
    pub previous_scope: Option<String>,
    pub new_scope: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAuditEntry {
    pub user_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub action: String,
    pub previous_scope: Option<String>,
    pub new_scope: Option<String>,
    pub ip_address: Option<String>,
}

pub async fn create(pool: &PgPool, input: &CreateAuditEntry) -> Result<AuditEntry, AppError> {
    // ip_address is stored as INET in the DB, but we accept it as a string
    let row = sqlx::query_as::<_, AuditEntry>(
        "INSERT INTO consent_audit_log (user_id, session_id, action, previous_scope, new_scope, ip_address)
         VALUES ($1, $2, $3, $4, $5, $6::inet)
         RETURNING id, user_id, session_id, action, previous_scope, new_scope, timestamp, ip_address::text"
    )
    .bind(input.user_id)
    .bind(input.session_id)
    .bind(&input.action)
    .bind(&input.previous_scope)
    .bind(&input.new_scope)
    .bind(&input.ip_address)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

pub async fn list_by_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<AuditEntry>, AppError> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        "SELECT id, user_id, session_id, action, previous_scope, new_scope, timestamp, ip_address::text
         FROM consent_audit_log WHERE user_id = $1 ORDER BY timestamp DESC"
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
