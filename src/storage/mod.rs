//! Storage backends for station snapshots.
//!
//! The Go original models this with a `DataStore` interface plus runtime type
//! assertions to discover whether a backend also supports history. Rust gets
//! the same behaviour from a single enum: the capability check becomes an
//! exhaustive `match`, and "this backend cannot do that" becomes an explicit
//! [`StoreError::Unsupported`] that the web layer turns into a 501.

pub mod r2;
pub mod sqlite;
pub mod tsv;
pub mod tsvfmt;

use anyhow::anyhow;
use chrono::{DateTime, Utc};

pub use r2::R2Storage;
pub use sqlite::SqliteStorage;
pub use tsv::TsvStorage;
pub use tsvfmt::TSV_HEADER;

use crate::tfl::Station;

/// A snapshot in time with aggregate statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoricalDataPoint {
    pub timestamp: DateTime<Utc>,
    pub total_bikes: i64,
    pub total_ebikes: i64,
    pub total_empty_docks: i64,
    pub station_count: i64,
}

/// Errors returned by a [`Store`].
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The active backend does not implement this operation.
    #[error("{0}")]
    Unsupported(&'static str),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

type StoreResult<T> = Result<T, StoreError>;

/// The storage backend the application is running against.
#[derive(Debug, Clone)]
pub enum Store {
    /// Local TSV files, used for development.
    Local(TsvStorage),
    /// Cloudflare R2, used in production.
    R2(R2Storage),
    /// Local SQLite file — POC for a future Cloudflare D1 backend.
    Sqlite(SqliteStorage),
}

impl Store {
    /// A human-readable name for the active backend.
    pub fn name(&self) -> &'static str {
        match self {
            Store::Local(_) => "local files",
            Store::R2(_) => "Cloudflare R2",
            Store::Sqlite(_) => "local SQLite (D1 POC)",
        }
    }

    /// Reads the most recent station data.
    pub async fn read_latest_stations(&self) -> StoreResult<(Vec<Station>, DateTime<Utc>)> {
        match self {
            Store::Local(store) => {
                let store = store.clone();
                blocking(move || store.read_latest_stations()).await
            }
            Store::R2(store) => Ok(store.read_latest_stations().await?),
            Store::Sqlite(store) => {
                let store = store.clone();
                blocking(move || store.read_latest_stations()).await
            }
        }
    }

    /// Returns all available data timestamps, newest first.
    pub async fn list_available_timestamps(&self) -> StoreResult<Vec<DateTime<Utc>>> {
        match self {
            Store::Local(store) => {
                let store = store.clone();
                blocking(move || store.list_available_timestamps()).await
            }
            Store::R2(store) => Ok(store.list_available_timestamps().await?),
            Store::Sqlite(store) => {
                let store = store.clone();
                blocking(move || store.list_available_timestamps()).await
            }
        }
    }

    /// Returns aggregate statistics for all available snapshots, newest first.
    ///
    /// The local TSV backend doesn't keep a full history, so it reports this
    /// as unsupported. R2 and SQLite both support it, though SQLite reads it
    /// from a precomputed table rather than downloading and summing every
    /// historical file.
    pub async fn get_historical_data(&self) -> StoreResult<Vec<HistoricalDataPoint>> {
        match self {
            Store::Local(_) => Err(StoreError::Unsupported(
                "Historical data not available with current storage backend",
            )),
            Store::R2(store) => Ok(store.get_historical_data().await?),
            Store::Sqlite(store) => {
                let store = store.clone();
                blocking(move || store.get_historical_data()).await
            }
        }
    }

    /// Returns station data for the snapshot closest to `target`.
    pub async fn get_snapshot_by_timestamp(
        &self,
        target: DateTime<Utc>,
    ) -> StoreResult<Vec<Station>> {
        match self {
            Store::Local(_) => Err(StoreError::Unsupported(
                "Historical snapshot data not available with current storage backend",
            )),
            Store::R2(store) => Ok(store.get_snapshot_by_timestamp(target).await?),
            Store::Sqlite(store) => {
                let store = store.clone();
                blocking(move || store.get_snapshot_by_timestamp(target)).await
            }
        }
    }
}

/// Runs a blocking filesystem read off the async runtime's worker threads.
async fn blocking<T, F>(task: F) -> StoreResult<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|err| StoreError::Other(anyhow!("storage task panicked: {err}")))?
        .map_err(StoreError::Other)
}
