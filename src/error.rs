//! Application-level error type.
//!
//! All handlers return `Result<_, AppError>`; `IntoResponse` collapses the
//! enum to a well-formed JSON error body with the right HTTP status.
//!
//! `anyhow` is intentionally absent — every failure mode is enumerated.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::ids::ETag;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    // --- client-visible failure modes -------------------------------------
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("gone: {0}")]
    Gone(String),

    #[error("payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("if-match precondition failed")]
    PreconditionFailed { current_etag: ETag },

    #[error("missing header: {0}")]
    MissingHeader(&'static str),

    #[error("invalid pseudo_id format: expected 24 hex chars")]
    InvalidPseudoId,

    #[error("illegal session state transition: {from} -> {to}")]
    IllegalTransition { from: String, to: String },

    // --- infra / upstream failures ----------------------------------------
    #[error("database error")]
    Database(#[from] sqlx::Error),

    #[error("s3 error: {reason}")]
    S3 { reason: String, retryable: bool },

    #[error("serde error: {0}")]
    Serde(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn s3(msg: impl Into<String>) -> Self {
        Self::S3 {
            reason: msg.into(),
            retryable: true,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, json!({"error": m})),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, json!({"error": m})),
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, json!({"error": m})),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, json!({"error": m})),
            AppError::Conflict(m) => (StatusCode::CONFLICT, json!({"error": m})),
            AppError::Gone(m) => (StatusCode::GONE, json!({"error": m})),
            AppError::PayloadTooLarge(m) => (StatusCode::PAYLOAD_TOO_LARGE, json!({"error": m})),
            AppError::PreconditionFailed { current_etag } => (
                StatusCode::PRECONDITION_FAILED,
                json!({
                    "error": "if-match precondition failed",
                    "current_etag": current_etag.as_str(),
                }),
            ),
            AppError::MissingHeader(h) => (
                StatusCode::BAD_REQUEST,
                json!({"error": format!("missing required header: {h}")}),
            ),
            AppError::InvalidPseudoId => (
                StatusCode::BAD_REQUEST,
                json!({"error": "invalid pseudo_id format: expected 24 hex chars"}),
            ),
            AppError::IllegalTransition { from, to } => (
                StatusCode::CONFLICT,
                json!({"error": format!("illegal transition: {from} -> {to}")}),
            ),
            AppError::Database(e) => {
                tracing::error!(error = %e, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error": "database error"}),
                )
            }
            AppError::S3 { reason, retryable } => {
                tracing::error!(%reason, retryable, "s3 error");
                (
                    StatusCode::BAD_GATEWAY,
                    json!({"error": format!("s3: {reason}"), "retryable": retryable}),
                )
            }
            AppError::Serde(m) => (
                StatusCode::BAD_REQUEST,
                json!({"error": format!("invalid json: {m}")}),
            ),
            AppError::Internal(m) => {
                tracing::error!(error = %m, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error": "internal error"}),
                )
            }
        };

        (status, axum::Json(body)).into_response()
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Serde(e.to_string())
    }
}

impl From<crate::ids::PseudoIdError> for AppError {
    fn from(_: crate::ids::PseudoIdError) -> Self {
        AppError::InvalidPseudoId
    }
}

impl From<crate::state::UnknownStatus> for AppError {
    fn from(e: crate::state::UnknownStatus) -> Self {
        AppError::BadRequest(format!("unknown status: {}", e.0))
    }
}
