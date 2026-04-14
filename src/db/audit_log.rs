//! Append-only audit log. Every mutating handler writes one row.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditRow {
    pub id: Uuid,
    pub at_ts: DateTime<Utc>,
    pub actor_service: String,
    pub actor_pseudo: Option<String>,
    pub session_id: Option<Uuid>,
    pub resource_type: String,
    pub resource_id: String,
    pub action: String,
    pub detail: Option<serde_json::Value>,
}

pub struct Entry<'a> {
    pub actor_service: &'a str,
    pub actor_pseudo: Option<&'a str>,
    pub session_id: Option<Uuid>,
    pub resource_type: &'a str,
    pub resource_id: String,
    pub action: &'a str,
    pub detail: Option<serde_json::Value>,
}

pub async fn append(pool: &PgPool, e: &Entry<'_>) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO audit_log
            (actor_service, actor_pseudo, session_id, resource_type, resource_id, action, detail)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(e.actor_service)
    .bind(e.actor_pseudo)
    .bind(e.session_id)
    .bind(e.resource_type)
    .bind(&e.resource_id)
    .bind(e.action)
    .bind(&e.detail)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn append_tx(
    tx: &mut Transaction<'_, Postgres>,
    e: &Entry<'_>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO audit_log
            (actor_service, actor_pseudo, session_id, resource_type, resource_id, action, detail)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(e.actor_service)
    .bind(e.actor_pseudo)
    .bind(e.session_id)
    .bind(e.resource_type)
    .bind(&e.resource_id)
    .bind(e.action)
    .bind(&e.detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct QueryFilter {
    pub session_id: Option<Uuid>,
    pub resource_type: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

pub async fn query(pool: &PgPool, f: &QueryFilter) -> Result<Vec<AuditRow>, AppError> {
    let rows = sqlx::query_as::<_, AuditRow>(
        "SELECT * FROM audit_log
         WHERE ($1::uuid IS NULL OR session_id = $1::uuid)
           AND ($2::text IS NULL OR resource_type = $2::text)
           AND ($3::timestamptz IS NULL OR at_ts >= $3::timestamptz)
         ORDER BY at_ts DESC
         LIMIT COALESCE($4, 200)",
    )
    .bind(f.session_id)
    .bind(&f.resource_type)
    .bind(f.since)
    .bind(f.limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
