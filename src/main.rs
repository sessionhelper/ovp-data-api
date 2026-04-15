//! chronicle-data-api binary entry point.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;
use tracing::info;
use tracing_subscriber::EnvFilter;

use chronicle_data_api::auth::spawn_reaper;
use chronicle_data_api::config::Config;
use chronicle_data_api::db;
use chronicle_data_api::events::create_bus;
use chronicle_data_api::metrics::Metrics;
use chronicle_data_api::routes::{build_router, AppState};
use chronicle_data_api::storage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("chronicle_data_api=info,tower_http=info")),
        )
        .init();

    let config = Config::from_env()?;
    info!(bind = %config.bind_addr, "starting chronicle-data-api");

    let pool = db::connect(&config.database_url).await?;
    db::run_migrations(&pool).await?;

    let store = Arc::new(storage::s3::new(&config).await);
    let _reaper = spawn_reaper(pool.clone(), config.heartbeat_reap);
    let events = create_bus();
    let metrics = Metrics::new();

    let state = AppState {
        pool,
        store,
        shared_secret: config.shared_secret.clone(),
        events,
        resume_ttl: config.resume_ttl,
        ws_queue_depth: config.ws_queue_depth,
        heartbeat_reap: config.heartbeat_reap,
        metrics,
    };
    let app = build_router(state);
    let service: IntoMakeServiceWithConnectInfo<_, SocketAddr> =
        app.into_make_service_with_connect_info::<SocketAddr>();

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    info!(addr = %config.bind_addr, "listening");
    axum::serve(listener, service).await?;
    Ok(())
}
