//! Collects station data from the TFL feed into local TSV files.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

use city_cycling::storage::TsvStorage;
use city_cycling::{init_tracing, parse_interval, shutdown_signal, tfl};

#[derive(Debug, Parser)]
#[command(
    name = "collector",
    about = "Fetch Santander Cycles station data into local TSV files"
)]
struct Args {
    /// Directory to store TSV files
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,

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
    let store = TsvStorage::new(&args.data_dir);

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

async fn fetch_and_store(client: &tfl::Client, store: &TsvStorage) -> Result<()> {
    tracing::info!("Fetching station data...");

    let stations = client.fetch_stations().await?;
    let path = store.write_stations(&stations)?;

    tracing::info!(
        "Saved {} stations to {}",
        stations.stations.len(),
        path.display()
    );

    Ok(())
}
