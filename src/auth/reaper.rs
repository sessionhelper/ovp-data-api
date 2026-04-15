//! Background reaper for stale service-session rows.
//!
//! Reap cadence = heartbeat window. We delete outright — there's no value in
//! a "dead but remembered" state since nothing reads expired tokens.

use std::time::Duration;

use sqlx::PgPool;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::db::service_sessions;

pub fn spawn_reaper(pool: PgPool, window: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Poll at 1/3 the window; small enough to keep rows fresh,
        // large enough to keep the query count negligible.
        let period = window / 3;
        let mut ticker = tokio::time::interval(period.max(Duration::from_secs(5)));
        loop {
            ticker.tick().await;
            match service_sessions::reap_stale(&pool, window).await {
                Ok(0) => {}
                Ok(n) => info!(removed = n, "reaped stale service sessions"),
                Err(e) => warn!(error = %e, "session reaper failed"),
            }
        }
    })
}
