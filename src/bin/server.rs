//! Serves the interactive map and the station/history JSON APIs.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use city_cycling::storage::{R2Storage, SqliteStorage, Store, TsvStorage};
use city_cycling::{config, init_tracing, shutdown_signal, tfl, web};

#[derive(Debug, Parser)]
#[command(
    name = "server",
    about = "Serve the Santander Cycles map and JSON APIs"
)]
struct Args {
    /// HTTP server port
    #[arg(long, default_value_t = 8080)]
    port: u16,

    /// Directory containing TSV data files (local mode only)
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,

    /// Use Cloudflare R2 for data storage; pass `--r2 false` for local files
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    r2: bool,

    /// Path to a local SQLite database file (POC for a future Cloudflare D1
    /// backend). When set, this takes priority over --r2 / local TSV files.
    #[arg(long)]
    sqlite: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_tracing();

    // Load .env before reading USE_R2/PORT so local runs can set them there too.
    let _ = dotenvy::dotenv();

    let use_r2 = args.r2 || env_set("USE_R2");
    let sqlite_db = args
        .sqlite
        .or_else(|| std::env::var("SQLITE_DB").ok().filter(|v| !v.trim().is_empty()).map(PathBuf::from));
    let port = match std::env::var("PORT").ok().filter(|v| !v.is_empty()) {
        Some(raw) => raw
            .trim()
            .parse()
            .with_context(|| format!("invalid PORT value {raw:?}"))?,
        None => args.port,
    };

    let store = if let Some(db_path) = sqlite_db {
        tracing::info!("Using local SQLite storage (POC for Cloudflare D1)");
        tracing::info!("Database file: {}", db_path.display());

        Store::Sqlite(
            SqliteStorage::open(&db_path)
                .with_context(|| format!("failed to open database {}", db_path.display()))?,
        )
    } else if use_r2 {
        tracing::info!("Using Cloudflare R2 for data storage");

        let cfg = config::load_r2_config().context("failed to load R2 config")?;
        tracing::info!("R2 Bucket: {}", cfg.bucket_name);

        Store::R2(R2Storage::new(
            &cfg.access_key_id,
            &cfg.secret_access_key,
            &cfg.endpoint,
            &cfg.bucket_name,
            &cfg.region,
            &cfg.prefix,
        ))
    } else {
        tracing::info!("Using local file storage");
        tracing::info!("Data directory: {}", args.data_dir.display());

        Store::Local(TsvStorage::new(&args.data_dir))
    };

    let app = web::router(store, tfl::Client::new());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;

    tracing::info!("Starting server on http://localhost:{port}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let signal = shutdown_signal().await;
            tracing::info!("Received signal {signal}, shutting down");
        })
        .await
        .context("server failed")?;

    Ok(())
}

/// True when an environment variable is set to a non-empty value.
fn env_set(key: &str) -> bool {
    std::env::var(key)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}
