//! Collects station data from the TFL feed into a local SQLite database.
//!
//! This is the POC collector for a future Cloudflare D1 backend — same
//! fetch/store loop as `collector` and `collector-r2`, writing instead into
//! the schema described in `storage::sqlite`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

use city_cycling::storage::SqliteStorage;
use city_cycling::{init_tracing, parse_interval, shutdown_signal, tfl};

#[derive(Debug, Parser)]
#[command(
    name = "collector-sqlite",
    about = "Fetch Santander Cycles station data into a local SQLite database"
)]
struct Args {
    /// Path to the SQLite database file (created, along with any parent
    /// directories, if it doesn't exist)
    #[arg(long, default_value = "data/city-cycling.db")]
    db: PathBuf,

    /// Fetch interval, e.g. 5m or 90s (set to 0 for one-shot mode)
    #[arg(long, default_value = "5m", value_parser = parse_interval)]
    interval: Duration,

    /// Run once and exit
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_tracing();

    let client = tfl::Client::new();
    let store = SqliteStorage::open(&args.db)
        .with_context(|| format!("failed to open database {}", args.db.display()))?;

    tracing::info!("Database file: {}", args.db.display());

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

async fn fetch_and_store(client: &tfl::Client, store: &SqliteStorage) -> Result<()> {
    tracing::info!("Fetching station data...");

    let stations = client.fetch_stations().await?;
    let count = stations.stations.len();

    // write_stations is blocking (rusqlite); keep it off the async runtime's
    // worker threads.
    let store = store.clone();
    let snapshot_id = tokio::task::spawn_blocking(move || store.write_stations(&stations))
        .await
        .context("blocking storage task panicked")??;

    tracing::info!("Saved {count} stations to SQLite (snapshot_id={snapshot_id})");

    Ok(())
}
