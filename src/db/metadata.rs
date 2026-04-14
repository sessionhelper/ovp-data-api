//! `session_metadata` — the per-session JSONB blob with ETag versioning.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::ids::ETag;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MetadataBlob {
    pub session_id: Uuid,
    pub blob: serde_json::Value,
    pub etag: ETag,
    pub updated_at: DateTime<Utc>,
}

pub async fn get(pool: &PgPool, session_id: Uuid) -> Result<MetadataBlob, AppError> {
    sqlx::query_as::<_, MetadataBlob>(
        "SELECT session_id, blob, etag, updated_at FROM session_metadata WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("metadata for session {session_id} not found")))
}

pub async fn get_opt(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<MetadataBlob>, AppError> {
    let row = sqlx::query_as::<_, MetadataBlob>(
        "SELECT session_id, blob, etag, updated_at FROM session_metadata WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn replace(
    pool: &PgPool,
    session_id: Uuid,
    blob: serde_json::Value,
) -> Result<MetadataBlob, AppError> {
    let etag = ETag::new_random();
    let row = sqlx::query_as::<_, MetadataBlob>(
        "INSERT INTO session_metadata (session_id, blob, etag, updated_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (session_id) DO UPDATE SET
            blob = EXCLUDED.blob,
            etag = EXCLUDED.etag,
            updated_at = NOW()
         RETURNING session_id, blob, etag, updated_at",
    )
    .bind(session_id)
    .bind(blob)
    .bind(etag.as_str())
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Shallow-merge `patch` into the existing blob, only if the existing ETag
/// matches `expected`. Returns 412 via `AppError::PreconditionFailed` when
/// the tag is stale.
pub async fn shallow_merge(
    pool: &PgPool,
    session_id: Uuid,
    expected: Option<&ETag>,
    patch: serde_json::Value,
) -> Result<MetadataBlob, AppError> {
    let mut tx = pool.begin().await?;

    let existing = sqlx::query_as::<_, MetadataBlob>(
        "SELECT session_id, blob, etag, updated_at
         FROM session_metadata
         WHERE session_id = $1
         FOR UPDATE",
    )
    .bind(session_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (base, current_etag) = match existing {
        Some(r) => (r.blob, r.etag),
        None => (serde_json::json!({}), ETag::new_random()),
    };

    if let Some(exp) = expected {
        if *exp != current_etag {
            return Err(AppError::PreconditionFailed {
                current_etag,
            });
        }
    }

    let merged = merge_shallow(base, patch)?;
    let new_etag = ETag::new_random();
    let row = sqlx::query_as::<_, MetadataBlob>(
        "INSERT INTO session_metadata (session_id, blob, etag, updated_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (session_id) DO UPDATE SET
            blob = EXCLUDED.blob,
            etag = EXCLUDED.etag,
            updated_at = NOW()
         RETURNING session_id, blob, etag, updated_at",
    )
    .bind(session_id)
    .bind(merged)
    .bind(new_etag.as_str())
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

fn merge_shallow(
    base: serde_json::Value,
    patch: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let mut base_obj = match base {
        serde_json::Value::Object(m) => m,
        serde_json::Value::Null => serde_json::Map::new(),
        _ => {
            return Err(AppError::BadRequest(
                "existing metadata blob is not an object — cannot shallow-merge".to_string(),
            ))
        }
    };
    let patch_obj = match patch {
        serde_json::Value::Object(m) => m,
        _ => {
            return Err(AppError::BadRequest(
                "patch body must be a JSON object".to_string(),
            ))
        }
    };
    for (k, v) in patch_obj {
        base_obj.insert(k, v);
    }
    Ok(serde_json::Value::Object(base_obj))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shallow_merge_replaces_top_level_keys_only() {
        let base = json!({ "a": 1, "b": { "x": 1 } });
        let patch = json!({ "b": { "y": 2 }, "c": 3 });
        let got = merge_shallow(base, patch).unwrap();
        // Note: `b` is replaced wholesale, nested key `x` is gone.
        assert_eq!(got, json!({ "a": 1, "b": { "y": 2 }, "c": 3 }));
    }

    #[test]
    fn shallow_merge_rejects_non_object_patch() {
        let base = json!({});
        let patch = json!([1, 2, 3]);
        assert!(merge_shallow(base, patch).is_err());
    }
}
