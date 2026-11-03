//! Local filesystem storage for station snapshots.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};

use super::tsvfmt;
use crate::tfl::{Station, Stations};

/// Reads and writes station data as TSV files in a local directory.
#[derive(Debug, Clone)]
pub struct TsvStorage {
    data_dir: PathBuf,
}

impl TsvStorage {
    /// Creates a new TSV storage instance rooted at `data_dir`.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Writes station data to a timestamped TSV file and returns its path.
    pub fn write_stations(&self, stations: &Stations) -> Result<PathBuf> {
        fs::create_dir_all(&self.data_dir).context("failed to create data directory")?;

        let timestamp = Utc::now();
        let path = self.data_dir.join(tsvfmt::snapshot_filename(timestamp));

        fs::write(&path, tsvfmt::render(&stations.stations, timestamp))
            .with_context(|| format!("failed to write {}", path.display()))?;

        Ok(path)
    }

    /// Reads the most recent TSV file and returns its stations.
    pub fn read_latest_stations(&self) -> Result<(Vec<Station>, DateTime<Utc>)> {
        let files = self.list_tsv_files()?;

        // Files are sorted newest first.
        let Some(latest) = files.first() else {
            bail!("no station data files found");
        };

        Self::read_tsv_file(latest)
    }

    /// Returns every timestamp for which data is available, newest first.
    pub fn list_available_timestamps(&self) -> Result<Vec<DateTime<Utc>>> {
        Ok(self
            .list_tsv_files()?
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(tsvfmt::parse_timestamp_from_key)
            })
            .collect())
    }

    /// Returns snapshot files sorted by timestamp (newest first).
    ///
    /// A missing data directory yields an empty list rather than an error, which
    /// keeps a fresh checkout working before the collector has ever run.
    fn list_tsv_files(&self) -> Result<Vec<PathBuf>> {
        let entries = match fs::read_dir(&self.data_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).context("failed to read data directory"),
        };

        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("stations_") && name.ends_with(".tsv"))
            })
            .collect();

        // The file name embeds the timestamp, so a reverse sort puts the newest first.
        files.sort();
        files.reverse();

        Ok(files)
    }

    /// Reads a single TSV file.
    fn read_tsv_file(path: &Path) -> Result<(Vec<Station>, DateTime<Utc>)> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to open {}", path.display()))?;

        let (stations, timestamp) = tsvfmt::parse(&content);

        // Fall back to the file name when the rows carry no usable timestamp.
        let timestamp = timestamp
            .or_else(|| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(tsvfmt::parse_timestamp_from_key)
            })
            .unwrap_or_else(Utc::now);

        Ok((stations, timestamp))
    }
}
