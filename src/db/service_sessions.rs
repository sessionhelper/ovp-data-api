//! Service-to-service session tokens (one per authenticated caller).

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ServiceSession {
    pub id: Uuid,
    pub service_name: String,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

pub async fn insert(
    pool: &PgPool,
    service_name: &str,
    token_hash: &str,
) -> Result<ServiceSession, AppError> {
    let row = sqlx::query_as::<_, ServiceSession>(
        "INSERT INTO service_sessions (service_name, token_hash)
         VALUES ($1, $2)
         ON CONFLICT (token_hash) DO UPDATE SET last_seen_at = NOW()
         RETURNING *",
    )
    .bind(service_name)
    .bind(token_hash)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Returns `Some(service_name)` if the token exists and isn't past the
/// heartbeat reap window.
pub async fn lookup_alive(
    pool: &PgPool,
    token_hash: &str,
    reap: Duration,
) -> Result<Option<String>, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        service_name: String,
    }
    let row = sqlx::query_as::<_, Row>(
        "SELECT service_name FROM service_sessions
         WHERE token_hash = $1 AND last_seen_at > NOW() - ($2::bigint || ' seconds')::interval",
    )
    .bind(token_hash)
    .bind(reap.as_secs() as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.service_name))
}

pub async fn touch(pool: &PgPool, token_hash: &str) -> Result<bool, AppError> {
    let res = sqlx::query(
        "UPDATE service_sessions SET last_seen_at = NOW() WHERE token_hash = $1",
    )
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn reap_stale(pool: &PgPool, window: Duration) -> Result<u64, AppError> {
    let res = sqlx::query(
        "DELETE FROM service_sessions
         WHERE last_seen_at < NOW() - ($1::bigint || ' seconds')::interval",
    )
    .bind(window.as_secs() as i64)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}
