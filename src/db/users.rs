use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub discord_id_hash: String,
    pub pseudo_id: String,
    pub global_opt_out: bool,
    pub opt_out_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUser {
    pub discord_id_hash: String,
    pub pseudo_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUser {
    pub global_opt_out: Option<bool>,
}

pub async fn upsert(pool: &PgPool, input: &CreateUser) -> Result<User, AppError> {
    let row = sqlx::query_as::<_, User>(
        "INSERT INTO users (discord_id_hash, pseudo_id)
         VALUES ($1, $2)
         ON CONFLICT (discord_id_hash)
         DO UPDATE SET pseudo_id = users.pseudo_id
         RETURNING *"
    )
    .bind(&input.discord_id_hash)
    .bind(&input.pseudo_id)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

pub async fn get_by_pseudo_id(pool: &PgPool, pseudo_id: &str) -> Result<User, AppError> {
    let row = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE pseudo_id = $1"
    )
    .bind(pseudo_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("user {pseudo_id} not found")))?;

    Ok(row)
}

pub async fn update(pool: &PgPool, pseudo_id: &str, input: &UpdateUser) -> Result<User, AppError> {
    let row = sqlx::query_as::<_, User>(
        "UPDATE users SET
            global_opt_out = COALESCE($2, global_opt_out),
            opt_out_at = CASE WHEN $2 = true THEN now() ELSE opt_out_at END
         WHERE pseudo_id = $1
         RETURNING *"
    )
    .bind(pseudo_id)
    .bind(input.global_opt_out)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("user {pseudo_id} not found")))?;

    Ok(row)
}

pub async fn delete(pool: &PgPool, pseudo_id: &str) -> Result<(), AppError> {
    let result = sqlx::query("DELETE FROM users WHERE pseudo_id = $1")
        .bind(pseudo_id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {pseudo_id} not found")));
    }

    Ok(())
}
