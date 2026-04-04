use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};

use crate::db::users as db;
use crate::error::AppError;
use crate::routes::AppState;

async fn create_user(
    State(state): State<AppState>,
    Json(input): Json<db::CreateUser>,
) -> Result<Json<db::User>, AppError> {
    let user = db::upsert(&state.pool, &input).await?;
    Ok(Json(user))
}

async fn get_user(
    State(state): State<AppState>,
    Path(pseudo_id): Path<String>,
) -> Result<Json<db::User>, AppError> {
    let user = db::get_by_pseudo_id(&state.pool, &pseudo_id).await?;
    Ok(Json(user))
}

async fn update_user(
    State(state): State<AppState>,
    Path(pseudo_id): Path<String>,
    Json(input): Json<db::UpdateUser>,
) -> Result<Json<db::User>, AppError> {
    let user = db::update(&state.pool, &pseudo_id, &input).await?;
    Ok(Json(user))
}

async fn delete_user(
    State(state): State<AppState>,
    Path(pseudo_id): Path<String>,
) -> Result<StatusCode, AppError> {
    db::delete(&state.pool, &pseudo_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/users", post(create_user))
        .route("/internal/users/{pseudo_id}", get(get_user).patch(update_user).delete(delete_user))
}
