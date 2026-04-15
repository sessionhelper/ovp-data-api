//! `/health/live` + `/health/ready`. Both are unauthenticated.

use axum::{extract::State, routing::get, Json, Router};
use serde_json::json;

use crate::routes::AppState;

async fn live() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn ready(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Poll both dependencies concurrently — they are independent.
    let (db_ok, s3_ok) = tokio::join!(
        async { sqlx::query("SELECT 1").fetch_one(&state.pool).await.is_ok() },
        state.store.head_bucket(),
    );
    let s3_ok = s3_ok.is_ok();
    Json(json!({
        "ok": db_ok && s3_ok,
        "db": db_ok,
        "s3": s3_ok,
    }))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
}
