//! Uniform CRUD for `segments`, `beats`, `scenes`.
//!
//! All three resources share the same row shape:
//!   id, session_id, client_id, start_ms, end_ms, text/title+summary,
//!   flags, original, etag, author_service, author_user_pseudo_id,
//!   created_at, updated_at.
//!
//! A single `ResourceTable` struct lets us parameterize the SQL by table
//! name + the natural-text columns. This collapses three near-identical
//! CRUD modules into one.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::ids::{ETag, PseudoId};

/// Natural-text columns for a resource. Segments use (`text`, `-`),
/// beats + scenes use (`title`, `summary`).
#[derive(Clone, Copy)]
pub enum TextShape {
    Segment,
    TitleSummary,
}

#[derive(Clone, Copy)]
pub struct ResourceTable {
    pub table: &'static str,
    pub shape: TextShape,
    pub resource_type: &'static str,
}

pub const SEGMENTS: ResourceTable = ResourceTable {
    table: "session_segments",
    shape: TextShape::Segment,
    resource_type: "segment",
};
pub const BEATS: ResourceTable = ResourceTable {
    table: "session_beats",
    shape: TextShape::TitleSummary,
    resource_type: "beat",
};
pub const SCENES: ResourceTable = ResourceTable {
    table: "session_scenes",
    shape: TextShape::TitleSummary,
    resource_type: "scene",
};

/// Outward-facing row. Fields that don't apply to a given resource come
/// back as `None` / empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniformRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub client_id: String,
    pub pseudo_id: Option<PseudoId>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub confidence: Option<f64>,
    pub flags: serde_json::Value,
    pub original: Option<serde_json::Value>,
    pub etag: ETag,
    pub author_service: String,
    pub author_user_pseudo_id: Option<PseudoId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Shared input for a single row. Callers pre-validate shape correctness;
/// e.g., segments must set `text`, beats/scenes must set `title`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateInput {
    pub client_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub pseudo_id: Option<PseudoId>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub flags: Option<serde_json::Value>,
    #[serde(default)]
    pub original: Option<serde_json::Value>,
    #[serde(default)]
    pub author_user_pseudo_id: Option<PseudoId>,
}

/// Response shape for bulk insert — reports which client_ids were inserted
/// vs deduplicated.
#[derive(Debug, Serialize)]
pub struct BulkResult {
    pub inserted: Vec<UniformRow>,
    pub deduplicated_client_ids: Vec<String>,
}

/// Partial update body for a single row.
#[derive(Debug, Clone, Deserialize)]
pub struct PatchInput {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub confidence: Option<Option<f64>>,
    #[serde(default)]
    pub start_ms: Option<i64>,
    #[serde(default)]
    pub end_ms: Option<i64>,
    #[serde(default)]
    pub flags: Option<serde_json::Value>,
    #[serde(default)]
    pub author_user_pseudo_id: Option<PseudoId>,
}

pub async fn bulk_insert(
    pool: &PgPool,
    rt: ResourceTable,
    session_id: Uuid,
    author_service: &str,
    inputs: &[CreateInput],
) -> Result<BulkResult, AppError> {
    if inputs.is_empty() {
        return Ok(BulkResult {
            inserted: Vec::new(),
            deduplicated_client_ids: Vec::new(),
        });
    }
    validate_inputs(rt, inputs)?;

    // We generate a fresh ETag per row rather than reusing one, so that
    // concurrent patches to different rows in the same batch still diverge.
    let mut tx = pool.begin().await?;

    // Find existing client_ids up-front so we can report dedup cleanly.
    let submitted: Vec<String> = inputs.iter().map(|i| i.client_id.clone()).collect();
    let existing_rows: Vec<String> = sqlx::query_scalar::<_, String>(&format!(
        "SELECT client_id FROM {} WHERE session_id = $1 AND client_id = ANY($2::text[])",
        rt.table,
    ))
    .bind(session_id)
    .bind(&submitted)
    .fetch_all(&mut *tx)
    .await?;

    let existing_set: std::collections::HashSet<String> = existing_rows.iter().cloned().collect();
    let to_insert: Vec<&CreateInput> = inputs
        .iter()
        .filter(|i| !existing_set.contains(&i.client_id))
        .collect();

    let mut inserted = Vec::with_capacity(to_insert.len());
    for inp in to_insert {
        let etag = ETag::new_random();
        let row = insert_row_in_tx(&mut tx, rt, session_id, author_service, inp, &etag).await?;
        inserted.push(row);
    }
    tx.commit().await?;

    Ok(BulkResult {
        inserted,
        deduplicated_client_ids: existing_rows,
    })
}

fn validate_inputs(rt: ResourceTable, inputs: &[CreateInput]) -> Result<(), AppError> {
    for i in inputs {
        match rt.shape {
            TextShape::Segment => {
                if i.text.is_none() {
                    return Err(AppError::BadRequest(format!(
                        "segments require `text` (client_id={})",
                        i.client_id
                    )));
                }
            }
            TextShape::TitleSummary => {
                if i.title.is_none() {
                    return Err(AppError::BadRequest(format!(
                        "{} rows require `title` (client_id={})",
                        rt.resource_type, i.client_id
                    )));
                }
            }
        }
        if i.end_ms < i.start_ms {
            return Err(AppError::BadRequest(format!(
                "end_ms < start_ms (client_id={})",
                i.client_id
            )));
        }
    }
    Ok(())
}

async fn insert_row_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    rt: ResourceTable,
    session_id: Uuid,
    author_service: &str,
    inp: &CreateInput,
    etag: &ETag,
) -> Result<UniformRow, AppError> {
    let sql = match rt.shape {
        TextShape::Segment => format!(
            "INSERT INTO {} (
                session_id, client_id, pseudo_id, start_ms, end_ms,
                text, confidence, flags, original, etag,
                author_service, author_user_pseudo_id
             )
             VALUES ($1,$2,$3,$4,$5,$6,$7,COALESCE($8,'{{}}'::jsonb),$9,$10,$11,$12)
             RETURNING *",
            rt.table
        ),
        TextShape::TitleSummary => format!(
            "INSERT INTO {} (
                session_id, client_id, start_ms, end_ms,
                title, summary, flags, original, etag,
                author_service, author_user_pseudo_id
             )
             VALUES ($1,$2,$3,$4,$5,$6,COALESCE($7,'{{}}'::jsonb),$8,$9,$10,$11)
             RETURNING *",
            rt.table
        ),
    };

    let q = sqlx::query(&sql);
    let q = q.bind(session_id).bind(&inp.client_id);

    let pg_row = match rt.shape {
        TextShape::Segment => q
            .bind(inp.pseudo_id.as_ref().map(|p| p.as_str()))
            .bind(inp.start_ms)
            .bind(inp.end_ms)
            .bind(inp.text.as_deref().unwrap_or(""))
            .bind(inp.confidence)
            .bind(inp.flags.clone())
            .bind(inp.original.clone())
            .bind(etag.as_str())
            .bind(author_service)
            .bind(inp.author_user_pseudo_id.as_ref().map(|p| p.as_str()))
            .fetch_one(&mut **tx)
            .await?,
        TextShape::TitleSummary => q
            .bind(inp.start_ms)
            .bind(inp.end_ms)
            .bind(inp.title.as_deref().unwrap_or(""))
            .bind(inp.summary.as_deref().unwrap_or(""))
            .bind(inp.flags.clone())
            .bind(inp.original.clone())
            .bind(etag.as_str())
            .bind(author_service)
            .bind(inp.author_user_pseudo_id.as_ref().map(|p| p.as_str()))
            .fetch_one(&mut **tx)
            .await?,
    };

    row_from_pg(rt, &pg_row)
}

pub async fn get(pool: &PgPool, rt: ResourceTable, id: Uuid) -> Result<UniformRow, AppError> {
    let sql = format!("SELECT * FROM {} WHERE id = $1", rt.table);
    let pg_row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{} {id} not found", rt.resource_type)))?;
    row_from_pg(rt, &pg_row)
}

pub async fn list_by_session(
    pool: &PgPool,
    rt: ResourceTable,
    session_id: Uuid,
    pseudo_id: Option<&PseudoId>,
    since_ms: Option<i64>,
) -> Result<Vec<UniformRow>, AppError> {
    let sql = format!(
        "SELECT * FROM {} WHERE session_id = $1
            AND ($2::text IS NULL OR pseudo_id = $2::text)
            AND ($3::bigint IS NULL OR start_ms >= $3)
         ORDER BY start_ms ASC, id ASC",
        rt.table
    );
    let rows = sqlx::query(&sql)
        .bind(session_id)
        .bind(pseudo_id.map(|p| p.as_str().to_string()))
        .bind(since_ms)
        .fetch_all(pool)
        .await?;
    let out: Vec<UniformRow> = rows
        .iter()
        .map(|r| row_from_pg(rt, r))
        .collect::<Result<_, _>>()?;
    Ok(out)
}

/// Patch a single row, requiring `If-Match` when `expected_etag` is set.
pub async fn patch(
    pool: &PgPool,
    rt: ResourceTable,
    id: Uuid,
    expected_etag: Option<&ETag>,
    patch: &PatchInput,
) -> Result<UniformRow, AppError> {
    let mut tx = pool.begin().await?;
    let sql_lock = format!("SELECT * FROM {} WHERE id = $1 FOR UPDATE", rt.table);
    let existing = sqlx::query(&sql_lock)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{} {id} not found", rt.resource_type)))?;
    let existing_row = row_from_pg(rt, &existing)?;

    if let Some(exp) = expected_etag {
        if *exp != existing_row.etag {
            return Err(AppError::PreconditionFailed {
                current_etag: existing_row.etag,
            });
        }
    }

    let new_etag = ETag::new_random();
    let patched = apply_patch(rt, id, &new_etag, patch, &existing_row);
    let pg_row = match rt.shape {
        TextShape::Segment => {
            let sql = format!(
                "UPDATE {} SET
                    start_ms = $2, end_ms = $3,
                    text = $4, confidence = $5,
                    flags = $6, etag = $7,
                    author_user_pseudo_id = COALESCE($8, author_user_pseudo_id),
                    updated_at = NOW()
                 WHERE id = $1
                 RETURNING *",
                rt.table
            );
            sqlx::query(&sql)
                .bind(id)
                .bind(patched.start_ms)
                .bind(patched.end_ms)
                .bind(patched.text.clone().unwrap_or_default())
                .bind(patched.confidence)
                .bind(patched.flags.clone())
                .bind(new_etag.as_str())
                .bind(patch.author_user_pseudo_id.as_ref().map(|p| p.as_str()))
                .fetch_one(&mut *tx)
                .await?
        }
        TextShape::TitleSummary => {
            let sql = format!(
                "UPDATE {} SET
                    start_ms = $2, end_ms = $3,
                    title = $4, summary = $5,
                    flags = $6, etag = $7,
                    author_user_pseudo_id = COALESCE($8, author_user_pseudo_id),
                    updated_at = NOW()
                 WHERE id = $1
                 RETURNING *",
                rt.table
            );
            sqlx::query(&sql)
                .bind(id)
                .bind(patched.start_ms)
                .bind(patched.end_ms)
                .bind(patched.title.clone().unwrap_or_default())
                .bind(patched.summary.clone().unwrap_or_default())
                .bind(patched.flags.clone())
                .bind(new_etag.as_str())
                .bind(patch.author_user_pseudo_id.as_ref().map(|p| p.as_str()))
                .fetch_one(&mut *tx)
                .await?
        }
    };
    let out = row_from_pg(rt, &pg_row)?;
    tx.commit().await?;
    Ok(out)
}

fn apply_patch(
    rt: ResourceTable,
    _id: Uuid,
    _etag: &ETag,
    p: &PatchInput,
    existing: &UniformRow,
) -> UniformRow {
    let mut out = existing.clone();
    if let Some(s) = p.start_ms {
        out.start_ms = s;
    }
    if let Some(e) = p.end_ms {
        out.end_ms = e;
    }
    if let Some(flags) = &p.flags {
        out.flags = flags.clone();
    }
    match rt.shape {
        TextShape::Segment => {
            if let Some(t) = &p.text {
                out.text = Some(t.clone());
            }
            if let Some(c) = p.confidence {
                out.confidence = c;
            }
        }
        TextShape::TitleSummary => {
            if let Some(t) = &p.title {
                out.title = Some(t.clone());
            }
            if let Some(s) = &p.summary {
                out.summary = Some(s.clone());
            }
        }
    }
    out
}

pub async fn delete(pool: &PgPool, rt: ResourceTable, id: Uuid) -> Result<UniformRow, AppError> {
    let sql = format!("DELETE FROM {} WHERE id = $1 RETURNING *", rt.table);
    let pg_row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{} {id} not found", rt.resource_type)))?;
    row_from_pg(rt, &pg_row)
}

/// Convert a raw Postgres row into a `UniformRow`.
fn row_from_pg(rt: ResourceTable, r: &sqlx::postgres::PgRow) -> Result<UniformRow, AppError> {
    let etag: String = r.try_get("etag").map_err(AppError::Database)?;
    let pseudo_raw: Option<String> = match rt.shape {
        TextShape::Segment => r.try_get("pseudo_id").ok(),
        _ => None,
    };
    let author_pseudo_raw: Option<String> = r.try_get("author_user_pseudo_id").ok();

    let pseudo_id = match pseudo_raw {
        Some(s) => Some(PseudoId::new(s)?),
        None => None,
    };
    let author_user_pseudo_id = match author_pseudo_raw {
        Some(s) => Some(PseudoId::new(s)?),
        None => None,
    };

    let (text, title, summary, confidence) = match rt.shape {
        TextShape::Segment => (
            r.try_get::<String, _>("text").ok(),
            None,
            None,
            r.try_get::<Option<f64>, _>("confidence").unwrap_or(None),
        ),
        TextShape::TitleSummary => (
            None,
            r.try_get::<String, _>("title").ok(),
            r.try_get::<String, _>("summary").ok(),
            None,
        ),
    };

    Ok(UniformRow {
        id: r.try_get("id").map_err(AppError::Database)?,
        session_id: r.try_get("session_id").map_err(AppError::Database)?,
        client_id: r.try_get("client_id").map_err(AppError::Database)?,
        pseudo_id,
        start_ms: r.try_get("start_ms").map_err(AppError::Database)?,
        end_ms: r.try_get("end_ms").map_err(AppError::Database)?,
        text,
        title,
        summary,
        confidence,
        flags: r.try_get("flags").map_err(AppError::Database)?,
        original: r.try_get("original").map_err(AppError::Database)?,
        etag: ETag(etag),
        author_service: r.try_get("author_service").map_err(AppError::Database)?,
        author_user_pseudo_id,
        created_at: r.try_get("created_at").map_err(AppError::Database)?,
        updated_at: r.try_get("updated_at").map_err(AppError::Database)?,
    })
}

/// Delete all rows under a session (for the cascade delete path).
pub async fn delete_all_for_session(
    tx: &mut Transaction<'_, Postgres>,
    rt: ResourceTable,
    session_id: Uuid,
) -> Result<u64, AppError> {
    let sql = format!("DELETE FROM {} WHERE session_id = $1", rt.table);
    let res = sqlx::query(&sql).bind(session_id).execute(&mut **tx).await?;
    Ok(res.rows_affected())
}

/// Delete rows authored for a specific participant (segment-scoped by pseudo_id).
pub async fn delete_for_participant_segments(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    pseudo_id: &PseudoId,
) -> Result<u64, AppError> {
    let res = sqlx::query(
        "DELETE FROM session_segments WHERE session_id = $1 AND pseudo_id = $2",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected())
}
