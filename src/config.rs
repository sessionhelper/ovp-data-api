use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub s3_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_bucket: String,
    pub admission_token_path: String,
    pub bind_addr: String,
    pub admission_rotation_secs: u64,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .expect("DATABASE_URL must be set"),
            s3_endpoint: env::var("S3_ENDPOINT")
                .expect("S3_ENDPOINT must be set"),
            s3_access_key: env::var("S3_ACCESS_KEY")
                .expect("S3_ACCESS_KEY must be set"),
            s3_secret_key: env::var("S3_SECRET_KEY")
                .expect("S3_SECRET_KEY must be set"),
            s3_bucket: env::var("S3_BUCKET")
                .unwrap_or_else(|_| "ttrpg-dataset-raw".to_string()),
            admission_token_path: env::var("ADMISSION_TOKEN_PATH")
                .unwrap_or_else(|_| "/var/run/ovp/admission-token".to_string()),
            bind_addr: env::var("BIND_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8001".to_string()),
            admission_rotation_secs: env::var("ADMISSION_ROTATION_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
        }
    }
}
