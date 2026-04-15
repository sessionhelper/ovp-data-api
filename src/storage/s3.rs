//! S3 object-store adapter.
//!
//! The trait keeps every call site `aws_*`-free; `MemStore` substitutes
//! in tests, `S3Store` runs in prod against any S3-compatible endpoint.
//! Backed by `rust-s3` — much lighter than `aws-sdk-s3` (the SDK pulled
//! in ~200 transitive deps for four operations).

use async_trait::async_trait;
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::Region;
use tracing::info;

use crate::config::Config;
use crate::error::AppError;

#[async_trait]
pub trait ObjectStore: Send + Sync + 'static {
    async fn put(&self, key: &str, body: Vec<u8>, content_type: &str) -> Result<(), AppError>;
    async fn get(&self, key: &str) -> Result<Vec<u8>, AppError>;
    /// Delete an explicit list of keys. Returns the count of keys reported
    /// deleted by the underlying store.
    async fn delete_many(&self, keys: &[String]) -> Result<u32, AppError>;
    async fn head_bucket(&self) -> Result<(), AppError>;
}

pub struct S3Store {
    bucket: Box<Bucket>,
}

impl S3Store {
    pub fn bucket_name(&self) -> &str {
        &self.bucket.name
    }
}

pub async fn new(config: &Config) -> S3Store {
    let region = Region::Custom {
        region: "auto".into(),
        endpoint: config.s3_endpoint.clone(),
    };
    let creds = Credentials::new(
        Some(&config.s3_access_key),
        Some(&config.s3_secret_key),
        None,
        None,
        None,
    )
    .expect("valid s3 credentials");

    // `path_style` because Hetzner Object Storage serves the bucket at
    // `<endpoint>/<bucket>/...` (path-style) rather than the AWS-classic
    // `<bucket>.<endpoint>/...` virtual-hosted form.
    let bucket = Bucket::new(&config.s3_bucket, region, creds)
        .expect("bucket builder")
        .with_path_style();

    info!(
        endpoint = %config.s3_endpoint,
        bucket = %config.s3_bucket,
        "s3 client initialized"
    );
    S3Store { bucket }
}

fn ok_status(code: u16) -> bool {
    (200..300).contains(&code)
}

#[async_trait]
impl ObjectStore for S3Store {
    async fn put(&self, key: &str, body: Vec<u8>, content_type: &str) -> Result<(), AppError> {
        let resp = self
            .bucket
            .put_object_with_content_type(key, &body, content_type)
            .await
            .map_err(|e| AppError::s3(format!("put {key}: {e}")))?;
        if !ok_status(resp.status_code()) {
            return Err(AppError::s3(format!(
                "put {key} → HTTP {}",
                resp.status_code()
            )));
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, AppError> {
        let resp = self
            .bucket
            .get_object(key)
            .await
            .map_err(|e| AppError::s3(format!("get {key}: {e}")))?;
        match resp.status_code() {
            c if ok_status(c) => Ok(resp.bytes().to_vec()),
            404 => Err(AppError::NotFound(format!("object {key} not found"))),
            c => Err(AppError::s3(format!("get {key} → HTTP {c}"))),
        }
    }

    async fn delete_many(&self, keys: &[String]) -> Result<u32, AppError> {
        let mut count = 0u32;
        for k in keys {
            let resp = self
                .bucket
                .delete_object(k)
                .await
                .map_err(|e| AppError::s3(format!("delete {k}: {e}")))?;
            if !ok_status(resp.status_code()) && resp.status_code() != 404 {
                return Err(AppError::s3(format!(
                    "delete {k} → HTTP {}",
                    resp.status_code()
                )));
            }
            count += 1;
        }
        Ok(count)
    }

    async fn head_bucket(&self) -> Result<(), AppError> {
        let (_, code) = self
            .bucket
            .head_object("/")
            .await
            .map_err(|e| AppError::s3(format!("head_bucket: {e}")))?;
        // 200 = bucket exists with index, 404 = bucket exists but no `/` object,
        // 403 = exists but path forbidden. Any of these prove the bucket is
        // reachable; only auth/network failures should surface as errors.
        if matches!(code, 200..=299 | 403 | 404) {
            Ok(())
        } else {
            Err(AppError::s3(format!("head_bucket → HTTP {code}")))
        }
    }
}
