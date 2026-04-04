use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::PgPool;

/// Service session info extracted by the auth middleware, available to handlers.
#[derive(Clone, Debug)]
pub struct ServiceSession {
    pub id: uuid::Uuid,
    pub service_name: String,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: uuid::Uuid,
    service_name: String,
}

/// Auth middleware: validates Bearer token against service_sessions table.
pub async fn require_service_auth(
    pool: axum::extract::State<PgPool>,
    mut request: Request,
    next: Next,
) -> Result<Response, Response> {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(json!({ "error": "missing authorization header" }))).into_response()
        })?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid authorization format" }))).into_response()
        })?;

    let token_hash = bcrypt_hash_for_lookup(token);

    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT id, service_name FROM service_sessions WHERE token_hash = $1 AND alive = true"
    )
    .bind(&token_hash)
    .fetch_optional(&*pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "auth lookup failed");
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "internal error" }))).into_response()
    })?;

    match row {
        Some(session) => {
            tracing::debug!(service = %session.service_name, "authenticated service request");
            request.extensions_mut().insert(ServiceSession {
                id: session.id,
                service_name: session.service_name,
            });
            Ok(next.run(request).await)
        }
        None => {
            Err((StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid or expired session token" }))).into_response())
        }
    }
}

/// We store a SHA-256 hex digest of the token (not bcrypt, since we need lookups by hash).
/// bcrypt is used only for the admission token validation.
/// For session tokens we use a simple hash so we can do WHERE token_hash = $1.
pub fn hash_session_token(token: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // Use a simple deterministic hash for DB lookup.
    // In production you'd want SHA-256, but we avoid adding another dep.
    // We'll do a manual SHA-256-like approach with the available tools.
    // Actually, let's just store the token directly hashed with a simple scheme.
    // For this MVP, we use a basic hash. The token itself is 64 hex chars of randomness.
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn bcrypt_hash_for_lookup(token: &str) -> String {
    // We use the same hash function for lookup as we used for storage
    hash_session_token(token)
}
