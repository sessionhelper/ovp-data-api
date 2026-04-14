//! S3 object store adapter.
//!
//! Separating the trait means tests can substitute an in-memory store and
//! the rest of the code never imports `aws_sdk_s3`.

use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
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
    client: Client,
    bucket: String,
}

impl S3Store {
    pub fn bucket(&self) -> &str {
        &self.bucket
    }
}

pub async fn new(config: &Config) -> S3Store {
    let creds = aws_sdk_s3::config::Credentials::new(
        &config.s3_access_key,
        &config.s3_secret_key,
        None,
        None,
        "env",
    );
    let s3_config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .endpoint_url(&config.s3_endpoint)
        .credentials_provider(creds)
        .region(aws_sdk_s3::config::Region::new("auto"))
        .force_path_style(true)
        .build();
    let client = Client::from_conf(s3_config);
    info!(endpoint = %config.s3_endpoint, bucket = %config.s3_bucket, "s3 client initialized");
    S3Store {
        client,
        bucket: config.s3_bucket.clone(),
    }
}

#[async_trait]
impl ObjectStore for S3Store {
    async fn put(&self, key: &str, body: Vec<u8>, content_type: &str) -> Result<(), AppError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(body))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| AppError::s3(format!("put {key}: {e}")))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, AppError> {
        let result = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                if matches!(e.as_service_error(), Some(GetObjectError::NoSuchKey(_))) {
                    return AppError::NotFound(format!("object {key} not found"));
                }
                AppError::s3(format!("get {key}: {e}"))
            })?;
        let bytes = result
            .body
            .collect()
            .await
            .map_err(|e| AppError::s3(format!("read body {key}: {e}")))?
            .into_bytes()
            .to_vec();
        Ok(bytes)
    }

    async fn delete_many(&self, keys: &[String]) -> Result<u32, AppError> {
        let mut count = 0u32;
        for k in keys {
            match self
                .client
                .delete_object()
                .bucket(&self.bucket)
                .key(k)
                .send()
                .await
            {
                Ok(_) => count += 1,
                Err(e) => return Err(AppError::s3(format!("delete {k}: {e}"))),
            }
        }
        Ok(count)
    }

    async fn head_bucket(&self) -> Result<(), AppError> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(|e| AppError::s3(format!("head_bucket: {e}")))?;
        Ok(())
    }
}
