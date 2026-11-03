//! Cloudflare R2 (S3-compatible) storage for station snapshots.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use chrono::{DateTime, SecondsFormat, Utc};
use tokio::sync::Semaphore;

use super::tsvfmt;
use super::HistoricalDataPoint;
use crate::tfl::{Station, Stations};

/// How many snapshots to download at once when aggregating history.
const HISTORY_CONCURRENCY: usize = 16;

/// Reads and writes station data to Cloudflare R2.
#[derive(Debug, Clone)]
pub struct R2Storage {
    client: Client,
    bucket: String,
    prefix: String,
}

impl R2Storage {
    /// Creates a new R2 storage instance.
    ///
    /// `prefix` is optional and defaults to `snapshots/`.
    pub fn new(
        access_key_id: &str,
        secret_access_key: &str,
        endpoint: &str,
        bucket: &str,
        region: &str,
        prefix: &str,
    ) -> Self {
        let prefix = if prefix.is_empty() {
            "snapshots/"
        } else {
            prefix
        };

        let credentials = Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            "city-cycling-static",
        );

        // R2 ignores the region but the SDK requires one, and R2 only serves
        // path-style addressing.
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .credentials_provider(credentials)
            .region(Region::new(region.to_string()))
            .endpoint_url(endpoint)
            .force_path_style(true)
            .build();

        Self {
            client: Client::from_conf(config),
            bucket: bucket.to_string(),
            prefix: prefix.to_string(),
        }
    }

    /// Writes station data to R2 as a timestamped TSV file and returns its key.
    pub async fn write_stations(&self, stations: &Stations) -> Result<String> {
        let start = Instant::now();

        let timestamp = Utc::now();
        let key = format!("{}{}", self.prefix, tsvfmt::snapshot_filename(timestamp));
        let body = tsvfmt::render(&stations.stations, timestamp);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(body.into_bytes()))
            .content_type("text/tab-separated-values")
            .metadata(
                "timestamp",
                timestamp.to_rfc3339_opts(SecondsFormat::Secs, true),
            )
            .metadata("stations", stations.stations.len().to_string())
            .send()
            .await
            .map_err(sdk_err)
            .context("failed to upload to R2")?;

        tracing::info!(
            "[R2] write_stations completed in {:.1?} (stations={})",
            start.elapsed(),
            stations.stations.len()
        );

        Ok(key)
    }

    /// Returns all snapshot keys in R2, sorted by timestamp (newest first).
    pub async fn list_snapshots(&self) -> Result<Vec<String>> {
        let start = Instant::now();

        let mut pages = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&self.prefix)
            .into_paginator()
            .send();

        let mut keys: Vec<String> = Vec::new();
        while let Some(page) = pages.next().await {
            let page = page.map_err(sdk_err).context("failed to list objects")?;
            keys.extend(
                page.contents()
                    .iter()
                    .filter_map(|obj| obj.key())
                    .map(String::from),
            );
        }

        // Keys are `{prefix}stations_YYYYMMDD_HHMMSS.tsv`, so reverse
        // lexicographic order is newest first.
        keys.sort();
        keys.reverse();

        tracing::info!(
            "[R2] list_snapshots completed in {:.1?} ({} keys)",
            start.elapsed(),
            keys.len()
        );

        Ok(keys)
    }

    /// Reads the most recent snapshot from R2.
    pub async fn read_latest_stations(&self) -> Result<(Vec<Station>, DateTime<Utc>)> {
        let start = Instant::now();

        let keys = self.list_snapshots().await?;
        let Some(latest) = keys.first() else {
            bail!("no snapshots found in R2 bucket");
        };

        let result = self.get_snapshot(latest).await;

        tracing::info!(
            "[R2] read_latest_stations completed in {:.1?}",
            start.elapsed()
        );

        result
    }

    /// Returns all available snapshot timestamps from R2, newest first.
    ///
    /// Timestamps come from the object keys, so this needs a single list call
    /// rather than one download per snapshot.
    pub async fn list_available_timestamps(&self) -> Result<Vec<DateTime<Utc>>> {
        Ok(self
            .list_snapshots()
            .await?
            .iter()
            .filter_map(|key| tsvfmt::parse_timestamp_from_key(key))
            .collect())
    }

    /// Downloads and parses a specific snapshot from R2.
    pub async fn get_snapshot(&self, key: &str) -> Result<(Vec<Station>, DateTime<Utc>)> {
        let start = Instant::now();

        let body = self.get_object(key).await?;
        let content = String::from_utf8_lossy(&body);

        let (stations, timestamp) = tsvfmt::parse(&content);
        if stations.is_empty() {
            bail!("snapshot {key} contained no station rows");
        }

        // Fall back to the key when the rows carry no usable timestamp.
        let timestamp = timestamp
            .or_else(|| tsvfmt::parse_timestamp_from_key(key))
            .ok_or_else(|| anyhow!("could not determine timestamp for snapshot {key}"))?;

        tracing::debug!(
            "[R2] get_snapshot completed in {:.1?} (key={key})",
            start.elapsed()
        );

        Ok((stations, timestamp))
    }

    /// Returns station data for the snapshot closest to `target`.
    ///
    /// Candidate timestamps are parsed from the object keys, so only the single
    /// winning snapshot is downloaded.
    pub async fn get_snapshot_by_timestamp(&self, target: DateTime<Utc>) -> Result<Vec<Station>> {
        let start = Instant::now();

        let keys = self
            .list_snapshots()
            .await
            .context("failed to list snapshots")?;
        if keys.is_empty() {
            bail!("no snapshots available");
        }

        let closest = keys
            .iter()
            .filter_map(|key| {
                let ts = tsvfmt::parse_timestamp_from_key(key)?;
                Some(((ts - target).abs(), key))
            })
            .min_by_key(|(diff, _)| *diff);

        let Some((diff, key)) = closest else {
            bail!("no matching snapshot found for timestamp");
        };

        tracing::info!(
            "[R2] get_snapshot_by_timestamp found closest key {key} (diff={})",
            humantime::format_duration(diff.to_std().unwrap_or_default())
        );

        let (stations, _) = self
            .get_snapshot(key)
            .await
            .context("failed to get snapshot")?;

        tracing::info!(
            "[R2] get_snapshot_by_timestamp completed in {:.1?} (target={})",
            start.elapsed(),
            target.to_rfc3339_opts(SecondsFormat::Secs, true)
        );

        Ok(stations)
    }

    /// Returns aggregate statistics for all available snapshots, newest first.
    ///
    /// Snapshots are downloaded concurrently; one that fails to load is logged
    /// and skipped so a single bad object cannot break the whole series.
    pub async fn get_historical_data(&self) -> Result<Vec<HistoricalDataPoint>> {
        let start = Instant::now();

        let keys = self.list_snapshots().await?;
        let permits = Arc::new(Semaphore::new(HISTORY_CONCURRENCY));
        let mut tasks = tokio::task::JoinSet::new();

        for key in keys {
            let store = self.clone();
            let permits = Arc::clone(&permits);

            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await;

                match store.get_snapshot(&key).await {
                    Ok((stations, timestamp)) => Some(HistoricalDataPoint {
                        timestamp,
                        total_bikes: stations.iter().map(|s| s.nb_bikes).sum(),
                        total_ebikes: stations.iter().map(|s| s.nb_ebikes).sum(),
                        total_empty_docks: stations.iter().map(|s| s.nb_empty_docks).sum(),
                        station_count: stations.len() as i64,
                    }),
                    Err(err) => {
                        tracing::warn!("Failed to read snapshot {key}: {err:#}");
                        None
                    }
                }
            });
        }

        let mut data_points = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Some(point)) => data_points.push(point),
                Ok(None) => {}
                Err(err) => tracing::warn!("Snapshot task failed: {err}"),
            }
        }

        // Tasks finish out of order; the API contract is newest first.
        data_points.sort_by_key(|point| std::cmp::Reverse(point.timestamp));

        tracing::info!(
            "[R2] get_historical_data completed in {:.1?} ({} points)",
            start.elapsed(),
            data_points.len()
        );

        Ok(data_points)
    }

    /// Deletes a specific snapshot from R2.
    pub async fn delete_snapshot(&self, key: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(sdk_err)
            .context("failed to delete object")?;

        Ok(())
    }

    /// Uploads an arbitrary object to R2.
    pub async fn put_object(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data))
            .content_type(content_type)
            .send()
            .await
            .map_err(sdk_err)
            .context("failed to upload object")?;

        Ok(())
    }

    /// Downloads an arbitrary object from R2.
    pub async fn get_object(&self, key: &str) -> Result<Vec<u8>> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(sdk_err)
            .with_context(|| format!("failed to get object {key}"))?;

        let bytes = output
            .body
            .collect()
            .await
            .with_context(|| format!("failed to read object {key}"))?;

        Ok(bytes.into_bytes().to_vec())
    }

    /// Checks that the configured bucket exists and is reachable.
    pub async fn bucket_exists(&self) -> Result<bool> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(sdk_err)
            .with_context(|| format!("failed to access bucket '{}'", self.bucket))?;

        Ok(true)
    }
}

/// Renders an AWS SDK error with its full source chain, which `Display` alone
/// reduces to an unhelpful "service error".
fn sdk_err<E>(err: E) -> anyhow::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    anyhow!("{}", DisplayErrorContext(&err))
}
