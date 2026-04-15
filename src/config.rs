//! Runtime configuration, pulled from environment variables.

use std::env;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub s3_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_bucket: String,
    pub shared_secret: String,
    pub bind_addr: String,
    pub resume_ttl: Duration,
    pub heartbeat_reap: Duration,
    pub ws_queue_depth: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required env var: {0}")]
    Missing(&'static str),
    #[error("invalid env var {var}: {reason}")]
    Invalid { var: &'static str, reason: String },
}

impl Config {
    /// Load config from environment, returning a typed error on any missing
    /// required variable or any parse failure.
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: require("DATABASE_URL")?,
            s3_endpoint: require("S3_ENDPOINT")?,
            s3_access_key: require("S3_ACCESS_KEY")?,
            s3_secret_key: require("S3_SECRET_KEY")?,
            s3_bucket: require("S3_BUCKET")?,
            shared_secret: require("SHARED_SECRET")?,
            bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8001".to_string()),
            resume_ttl: Duration::from_secs(parse_u64("RESUME_TTL_SECS", 86_400)?),
            heartbeat_reap: Duration::from_secs(parse_u64("HEARTBEAT_REAP_SECS", 90)?),
            ws_queue_depth: parse_u64("WS_QUEUE_DEPTH", 64)? as usize,
        })
    }
}

fn require(var: &'static str) -> Result<String, ConfigError> {
    env::var(var).map_err(|_| ConfigError::Missing(var))
}

fn parse_u64(var: &'static str, default: u64) -> Result<u64, ConfigError> {
    match env::var(var) {
        Err(_) => Ok(default),
        Ok(raw) => raw.parse::<u64>().map_err(|e| ConfigError::Invalid {
            var,
            reason: e.to_string(),
        }),
    }
}
