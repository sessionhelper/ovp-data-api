use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("s3 error: {0}")]
    S3(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Database(e) => {
                tracing::error!(error = %e, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error".to_string())
            }
            AppError::S3(msg) => {
                tracing::error!(error = %msg, "s3 error");
                (StatusCode::INTERNAL_SERVER_ERROR, "storage error".to_string())
            }
            AppError::Internal(msg) => {
                tracing::error!(error = %msg, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };

        let body = axum::Json(json!({ "error": message }));
        (status, body).into_response()
    }
}
