use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SegmentFlag {
    pub id: Uuid,
    pub segment_id: Uuid,
    pub flagged_by: Uuid,
    pub reason: String,
    pub flagged_at: DateTime<Utc>,
    pub reverted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SegmentEdit {
    pub id: Uuid,
    pub segment_id: Uuid,
    pub edited_by: Uuid,
    pub original_text: String,
    pub new_text: String,
    pub edited_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFlag {
    pub flagged_by: Uuid,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEdit {
    pub edited_by: Uuid,
    pub new_text: String,
}

pub async fn create_flag(pool: &PgPool, segment_id: Uuid, input: &CreateFlag) -> Result<SegmentFlag, AppError> {
    let reason = input.reason.as_deref().unwrap_or("private_info");
    let row = sqlx::query_as::<_, SegmentFlag>(
        "INSERT INTO segment_flags (segment_id, flagged_by, reason)
         VALUES ($1, $2, $3)
         RETURNING *"
    )
    .bind(segment_id)
    .bind(input.flagged_by)
    .bind(reason)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

pub async fn delete_flag(pool: &PgPool, segment_id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE segment_flags SET reverted_at = now() WHERE segment_id = $1 AND reverted_at IS NULL"
    )
    .bind(segment_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn create_edit(pool: &PgPool, segment_id: Uuid, input: &CreateEdit) -> Result<SegmentEdit, AppError> {
    // Get the current text from the segment to store as original_text in the edit
    let current_text: (String,) = sqlx::query_as(
        "SELECT text FROM transcript_segments WHERE id = $1"
    )
    .bind(segment_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("segment {segment_id} not found")))?;

    // Update the segment text
    sqlx::query("UPDATE transcript_segments SET text = $2 WHERE id = $1")
        .bind(segment_id)
        .bind(&input.new_text)
        .execute(pool)
        .await?;

    // Record the edit
    let row = sqlx::query_as::<_, SegmentEdit>(
        "INSERT INTO segment_edits (segment_id, edited_by, original_text, new_text)
         VALUES ($1, $2, $3, $4)
         RETURNING *"
    )
    .bind(segment_id)
    .bind(input.edited_by)
    .bind(&current_text.0)
    .bind(&input.new_text)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

pub async fn list_edits(pool: &PgPool, segment_id: Uuid) -> Result<Vec<SegmentEdit>, AppError> {
    let rows = sqlx::query_as::<_, SegmentEdit>(
        "SELECT * FROM segment_edits WHERE segment_id = $1 ORDER BY edited_at"
    )
    .bind(segment_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
