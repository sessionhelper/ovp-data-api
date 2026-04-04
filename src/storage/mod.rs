pub mod audio;
pub mod metadata;

use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use tracing::info;

use crate::config::Config;

pub async fn create_s3_client(config: &Config) -> Client {
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
    client
}
