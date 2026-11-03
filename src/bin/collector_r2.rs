//! Collects station data from the TFL feed and uploads it to Cloudflare R2.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;

use city_cycling::config;
use city_cycling::storage::R2Storage;
use city_cycling::{init_tracing, parse_interval, shutdown_signal, tfl};

#[derive(Debug, Parser)]
#[command(
    name = "collector-r2",
    about = "Fetch Santander Cycles station data and upload it to Cloudflare R2"
)]
struct Args {
    /// Fetch interval, e.g. 5m or 15m (set to 0 for one-shot mode)
    #[arg(long, default_value = "15m", value_parser = parse_interval)]
    interval: Duration,

    /// Run once and exit
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_tracing();

    // Load R2 configuration from .env or environment variables.
    let cfg = config::load_r2_config().context("configuration error")?;

    // Log configuration (without secrets).
    tracing::info!("R2 Configuration:");
    tracing::info!("  Endpoint: {}", cfg.endpoint);
    tracing::info!("  Bucket: {}", cfg.bucket_name);
    tracing::info!("  Region: {}", cfg.region);
    tracing::info!("  Prefix: {}", cfg.prefix);

    let client = tfl::Client::new();
    let store = R2Storage::new(
        &cfg.access_key_id,
        &cfg.secret_access_key,
        &cfg.endpoint,
        &cfg.bucket_name,
        &cfg.region,
        &cfg.prefix,
    );

    // Verify the bucket up front; this catches configuration problems early.
    tracing::info!("Verifying R2 bucket access...");
    if !store.bucket_exists().await.context("bucket verification failed")? {
        bail!(
            "bucket '{}' does not exist or is not accessible",
            cfg.bucket_name
        );
    }
    tracing::info!("Bucket verified successfully");

    fetch_and_store(&client, &store)
        .await
        .context("initial fetch failed")?;

    if args.once || args.interval.is_zero() {
        tracing::info!("One-shot mode: exiting after single fetch");
        return Ok(());
    }

    let mut ticker = tokio::time::interval(args.interval);
    // Tokio's first tick fires immediately; the initial fetch already happened.
    ticker.tick().await;

    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    tracing::info!(
        "Collector running with {} interval. Press Ctrl+C to stop.",
        humantime::format_duration(args.interval)
    );

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(err) = fetch_and_store(&client, &store).await {
                    tracing::error!("Fetch failed: {err:#}");
                }
            }
            signal = &mut shutdown => {
                tracing::info!("Received signal {signal}, shutting down");
                return Ok(());
            }
        }
    }
}

async fn fetch_and_store(client: &tfl::Client, store: &R2Storage) -> Result<()> {
    tracing::info!("Fetching station data...");

    let stations = client.fetch_stations().await?;
    let key = store.write_stations(&stations).await?;

    tracing::info!(
        "Uploaded {} stations to R2: {key}",
        stations.stations.len()
    );

    Ok(())
}
