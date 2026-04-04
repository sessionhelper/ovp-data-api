mod auth;
mod config;
mod db;
mod error;
mod routes;
mod storage;

use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::auth::{generate_token, write_token_file, AdmissionState};
use crate::config::Config;
use crate::routes::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ovp_data_api=info,tower_http=info")),
        )
        .init();

    let config = Config::from_env();
    info!(bind = %config.bind_addr, "starting ovp-data-api");

    // Connect to Postgres and run migrations
    let pool = db::connect(&config.database_url).await?;
    db::run_migrations(&pool).await?;

    // Initialize S3 client
    let s3_client = storage::create_s3_client(&config).await;

    // Generate initial admission token and write to file
    let initial_token = generate_token();
    write_token_file(&config.admission_token_path, &initial_token).await?;
    info!(path = %config.admission_token_path, "admission token written");

    let admission = AdmissionState::new(initial_token);

    // Spawn background tasks
    auth::spawn_rotation_task(
        admission.clone(),
        config.admission_token_path.clone(),
        config.admission_rotation_secs,
    );
    auth::spawn_session_reaper(pool.clone());

    // Build application state and router
    let state = AppState {
        pool,
        s3_client,
        s3_bucket: config.s3_bucket,
        admission,
    };

    let app = routes::build_router(state);

    // Bind to localhost only
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    info!(addr = %config.bind_addr, "listening");

    axum::serve(listener, app).await?;

    Ok(())
}
