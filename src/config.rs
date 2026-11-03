//! Configuration loading for the Cloudflare R2 backend.

use anyhow::{bail, Result};

/// Holds Cloudflare R2 configuration.
#[derive(Debug, Clone)]
pub struct R2Config {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub endpoint: String,
    pub bucket_name: String,
    pub prefix: String,
    pub region: String,
}

/// Loads R2 configuration from environment variables or a `.env` file.
///
/// For local development the `.env` file is loaded first; in production we rely
/// on environment variables set by the platform, so a missing file is not an
/// error.
pub fn load_r2_config() -> Result<R2Config> {
    // Ignore the error if the file doesn't exist (expected in production).
    let _ = dotenvy::dotenv();

    let get = |key: &str| {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };

    let access_key_id = get("S3_ACCESS_KEY_ID");
    let secret_access_key = get("S3_SECRET_ACCESS_KEY");
    let endpoint = get("S3_ENDPOINT");
    let bucket_name = get("S3_BUCKET_NAME");

    // Default prefix keeps every snapshot under one R2 "folder".
    let prefix = get("S3_PREFIX").unwrap_or_else(|| "snapshots/".to_string());

    // For Cloudflare R2 the region is irrelevant, but the AWS SDK requires one.
    let region = get("S3_REGION").unwrap_or_else(|| "auto".to_string());

    let missing: Vec<&str> = [
        ("S3_ACCESS_KEY_ID", &access_key_id),
        ("S3_SECRET_ACCESS_KEY", &secret_access_key),
        ("S3_ENDPOINT", &endpoint),
        ("S3_BUCKET_NAME", &bucket_name),
    ]
    .iter()
    .filter(|(_, value)| value.is_none())
    .map(|(name, _)| *name)
    .collect();

    if !missing.is_empty() {
        bail!("missing required environment variables: {:?}", missing);
    }

    Ok(R2Config {
        access_key_id: access_key_id.unwrap(),
        secret_access_key: secret_access_key.unwrap(),
        endpoint: endpoint.unwrap(),
        bucket_name: bucket_name.unwrap(),
        prefix,
        region,
    })
}
