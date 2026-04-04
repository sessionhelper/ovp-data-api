pub mod middleware;

use rand::Rng;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Holds the current and previous admission tokens.
#[derive(Clone)]
pub struct AdmissionState {
    inner: Arc<RwLock<AdmissionTokens>>,
}

struct AdmissionTokens {
    current: String,
    previous: Option<String>,
}

impl AdmissionState {
    pub fn new(initial_token: String) -> Self {
        Self {
            inner: Arc::new(RwLock::new(AdmissionTokens {
                current: initial_token,
                previous: None,
            })),
        }
    }

    /// Check if a token matches the current or previous admission token.
    pub async fn validate(&self, token: &str) -> bool {
        let tokens = self.inner.read().await;
        if tokens.current == token {
            return true;
        }
        if let Some(ref prev) = tokens.previous {
            if prev == token {
                return true;
            }
        }
        false
    }

    /// Rotate to a new token, keeping the old one as previous.
    pub async fn rotate(&self, new_token: String) {
        let mut tokens = self.inner.write().await;
        tokens.previous = Some(std::mem::replace(&mut tokens.current, new_token));
    }
}

/// Generate a cryptographically random token string (64 hex chars).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Write the admission token to the file, creating parent directories if needed.
pub async fn write_token_file(path: &str, token: &str) -> std::io::Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, token).await?;

    // Best-effort: set file permissions to 640 on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o640);
        tokio::fs::set_permissions(path, perms).await?;
    }

    Ok(())
}

/// Spawn the admission token rotation task.
pub fn spawn_rotation_task(
    state: AdmissionState,
    token_path: String,
    interval_secs: u64,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        // Skip the first tick (token was just written at startup)
        interval.tick().await;

        loop {
            interval.tick().await;
            let new_token = generate_token();
            if let Err(e) = write_token_file(&token_path, &new_token).await {
                warn!(error = %e, "failed to write rotated admission token");
                continue;
            }
            state.rotate(new_token).await;
            info!("admission token rotated");
        }
    });
}

/// Spawn the session reaper that marks stale sessions as dead.
pub fn spawn_session_reaper(pool: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let result = sqlx::query(
                "UPDATE service_sessions SET alive = false WHERE alive = true AND last_seen < now() - interval '90 seconds'"
            )
            .execute(&pool)
            .await;

            match result {
                Ok(res) => {
                    let count = res.rows_affected();
                    if count > 0 {
                        info!(count, "reaped stale service sessions");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "session reaper query failed");
                }
            }
        }
    });
}
