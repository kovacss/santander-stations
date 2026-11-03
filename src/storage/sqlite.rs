//! Local SQLite storage for station snapshots.
//!
//! This is a POC stand-in for a future Cloudflare D1 backend (D1 is
//! SQLite-compatible), aimed at fixing a real problem with the R2 backend:
//! `/api/history` there has to download and parse every historical snapshot
//! file to sum a handful of columns, which gets slow as the bucket grows.
//!
//! The schema instead precomputes those sums at write time into
//! `snapshot_aggregates`, so history becomes a flat, indexed scan:
//!
//! - `stations`             — mostly-static station metadata (id/name/coords)
//! - `snapshots`             — one row per collector run, keyed by timestamp
//! - `station_readings`      — per-station numbers for a given snapshot
//! - `snapshot_aggregates`   — per-snapshot totals, written once, read many
//!
//! The DDL below is plain, portable SQLite (no extensions, no
//! `WITHOUT ROWID`), so moving it to D1 later should mean swapping the
//! connection, not the SQL.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use super::HistoricalDataPoint;
use crate::tfl::{Station, Stations};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS stations (
    id      INTEGER PRIMARY KEY,
    name    TEXT NOT NULL,
    lat     REAL NOT NULL,
    long    REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    captured_at TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS idx_snapshots_captured_at ON snapshots (captured_at);

CREATE TABLE IF NOT EXISTS station_readings (
    snapshot_id       INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    station_id        INTEGER NOT NULL REFERENCES stations(id),
    nb_bikes          INTEGER NOT NULL,
    nb_standard_bikes INTEGER NOT NULL,
    nb_ebikes         INTEGER NOT NULL,
    nb_empty_docks    INTEGER NOT NULL,
    nb_docks          INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, station_id)
);

CREATE TABLE IF NOT EXISTS snapshot_aggregates (
    snapshot_id       INTEGER PRIMARY KEY REFERENCES snapshots(id) ON DELETE CASCADE,
    captured_at       TEXT NOT NULL,
    total_bikes       INTEGER NOT NULL,
    total_ebikes      INTEGER NOT NULL,
    total_empty_docks INTEGER NOT NULL,
    station_count     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_snapshot_aggregates_captured_at ON snapshot_aggregates (captured_at);
"#;

/// Reads and writes station data to a local SQLite file.
#[derive(Debug, Clone)]
pub struct SqliteStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Opens (creating if needed) a SQLite database at `path` and ensures the
    /// schema exists.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        Self::init(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Opens an in-memory database. Used by tests, where touching the
    /// filesystem would just add noise.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory sqlite db")?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init(conn: &Connection) -> Result<()> {
        // WAL lets the server keep reading while the collector writes.
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("failed to set journal_mode")?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .context("failed to enable foreign_keys")?;
        conn.execute_batch(SCHEMA).context("failed to apply schema")?;
        Ok(())
    }

    /// Locks the shared connection. A single `Mutex<Connection>` is fine for
    /// a POC: SQLite only allows one writer at a time anyway, and reads here
    /// are cheap indexed lookups.
    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Writes a full snapshot — station metadata, per-station readings and
    /// the precomputed aggregate — in a single transaction, and returns the
    /// new snapshot's row id.
    pub fn write_stations(&self, stations: &Stations) -> Result<i64> {
        let timestamp = Utc::now();
        let captured_at = timestamp.to_rfc3339_opts(SecondsFormat::Secs, true);

        let mut conn = self.lock();
        let tx = conn.transaction().context("failed to start transaction")?;

        tx.execute(
            "INSERT INTO snapshots (captured_at) VALUES (?1)",
            params![captured_at],
        )
        .context("failed to insert snapshot")?;
        let snapshot_id = tx.last_insert_rowid();

        {
            let mut upsert_station = tx.prepare(
                "INSERT INTO stations (id, name, lat, long) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     name = excluded.name, lat = excluded.lat, long = excluded.long",
            )?;
            let mut insert_reading = tx.prepare(
                "INSERT INTO station_readings
                     (snapshot_id, station_id, nb_bikes, nb_standard_bikes,
                      nb_ebikes, nb_empty_docks, nb_docks)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;

            for s in &stations.stations {
                upsert_station
                    .execute(params![s.id, s.name, s.lat, s.long])
                    .context("failed to upsert station metadata")?;
                insert_reading
                    .execute(params![
                        snapshot_id,
                        s.id,
                        s.nb_bikes,
                        s.nb_standard_bikes,
                        s.nb_ebikes,
                        s.nb_empty_docks,
                        s.nb_docks,
                    ])
                    .context("failed to insert station reading")?;
            }
        }

        let total_bikes: i64 = stations.stations.iter().map(|s| s.nb_bikes).sum();
        let total_ebikes: i64 = stations.stations.iter().map(|s| s.nb_ebikes).sum();
        let total_empty_docks: i64 = stations.stations.iter().map(|s| s.nb_empty_docks).sum();
        let station_count = stations.stations.len() as i64;

        tx.execute(
            "INSERT INTO snapshot_aggregates
                 (snapshot_id, captured_at, total_bikes, total_ebikes,
                  total_empty_docks, station_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot_id,
                captured_at,
                total_bikes,
                total_ebikes,
                total_empty_docks,
                station_count
            ],
        )
        .context("failed to insert snapshot aggregate")?;

        tx.commit().context("failed to commit snapshot")?;

        Ok(snapshot_id)
    }

    /// Reads the most recent snapshot's stations.
    pub fn read_latest_stations(&self) -> Result<(Vec<Station>, DateTime<Utc>)> {
        let conn = self.lock();

        let latest: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, captured_at FROM snapshots ORDER BY captured_at DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .context("failed to query latest snapshot")?;

        let Some((snapshot_id, captured_at)) = latest else {
            bail!("no snapshots found in SQLite database");
        };

        let stations = Self::stations_for_snapshot(&conn, snapshot_id)?;
        Ok((stations, parse_rfc3339(&captured_at)?))
    }

    /// Returns all available snapshot timestamps, newest first.
    pub fn list_available_timestamps(&self) -> Result<Vec<DateTime<Utc>>> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT captured_at FROM snapshots ORDER BY captured_at DESC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

        rows.map(|row| parse_rfc3339(&row?))
            .collect::<Result<Vec<_>>>()
    }

    /// Returns aggregate statistics for every snapshot, newest first.
    ///
    /// This is the point of the schema: a flat read of `snapshot_aggregates`,
    /// no joins and no per-request summation, unlike the R2 backend which
    /// has to download and parse every historical file to compute the same
    /// numbers.
    pub fn get_historical_data(&self) -> Result<Vec<HistoricalDataPoint>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT captured_at, total_bikes, total_ebikes, total_empty_docks, station_count
             FROM snapshot_aggregates
             ORDER BY captured_at DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;

        rows.map(|row| {
            let (captured_at, total_bikes, total_ebikes, total_empty_docks, station_count) = row?;
            Ok(HistoricalDataPoint {
                timestamp: parse_rfc3339(&captured_at)?,
                total_bikes,
                total_ebikes,
                total_empty_docks,
                station_count,
            })
        })
        .collect::<Result<Vec<_>>>()
    }

    /// Returns station data for the snapshot whose `captured_at` is closest
    /// to `target`.
    pub fn get_snapshot_by_timestamp(&self, target: DateTime<Utc>) -> Result<Vec<Station>> {
        let conn = self.lock();

        let closest: Option<i64> = conn
            .query_row(
                "SELECT id FROM snapshots
                 ORDER BY ABS(strftime('%s', captured_at) - ?1) ASC
                 LIMIT 1",
                params![target.timestamp()],
                |row| row.get(0),
            )
            .optional()
            .context("failed to query closest snapshot")?;

        let Some(snapshot_id) = closest else {
            bail!("no snapshots available");
        };

        Self::stations_for_snapshot(&conn, snapshot_id)
    }

    /// Joins `station_readings` back onto `stations` for one snapshot.
    fn stations_for_snapshot(conn: &Connection, snapshot_id: i64) -> Result<Vec<Station>> {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.lat, s.long,
                    r.nb_bikes, r.nb_standard_bikes, r.nb_ebikes, r.nb_empty_docks, r.nb_docks
             FROM station_readings r
             JOIN stations s ON s.id = r.station_id
             WHERE r.snapshot_id = ?1
             ORDER BY s.id",
        )?;

        let rows = stmt.query_map(params![snapshot_id], |row| {
            Ok(Station {
                id: row.get(0)?,
                name: row.get(1)?,
                lat: row.get(2)?,
                long: row.get(3)?,
                nb_bikes: row.get(4)?,
                nb_standard_bikes: row.get(5)?,
                nb_ebikes: row.get(6)?,
                nb_empty_docks: row.get(7)?,
                nb_docks: row.get(8)?,
                ..Station::default()
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read station readings")
    }
}

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| anyhow!("invalid timestamp {raw:?} in database: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(id: i64, nb_bikes: i64, nb_ebikes: i64, nb_empty_docks: i64) -> Station {
        Station {
            id,
            name: format!("Station {id}"),
            lat: 51.5,
            long: -0.1,
            nb_bikes,
            nb_standard_bikes: nb_bikes - nb_ebikes,
            nb_ebikes,
            nb_empty_docks,
            nb_docks: nb_bikes + nb_empty_docks,
            ..Station::default()
        }
    }

    #[test]
    fn round_trips_a_snapshot() {
        let store = SqliteStorage::open_in_memory().unwrap();

        let stations = Stations {
            stations: vec![station(1, 4, 1, 15), station(2, 0, 0, 10)],
            ..Stations::default()
        };
        store.write_stations(&stations).unwrap();

        let (read_back, _) = store.read_latest_stations().unwrap();
        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[0].id, 1);
        assert_eq!(read_back[0].nb_bikes, 4);
        assert_eq!(read_back[1].nb_empty_docks, 10);
    }

    #[test]
    fn history_is_newest_first_and_precomputed() {
        let store = SqliteStorage::open_in_memory().unwrap();

        store
            .write_stations(&Stations {
                stations: vec![station(1, 4, 1, 15)],
                ..Stations::default()
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100)); // force a distinct second
        store
            .write_stations(&Stations {
                stations: vec![station(1, 2, 1, 17)],
                ..Stations::default()
            })
            .unwrap();

        let history = store.get_historical_data().unwrap();
        assert_eq!(history.len(), 2);
        // Newest snapshot (bikes=2) first.
        assert_eq!(history[0].total_bikes, 2);
        assert_eq!(history[1].total_bikes, 4);
        assert!(history[0].timestamp > history[1].timestamp);
    }

    #[test]
    fn finds_closest_snapshot_by_timestamp() {
        let store = SqliteStorage::open_in_memory().unwrap();

        store
            .write_stations(&Stations {
                stations: vec![station(1, 4, 1, 15)],
                ..Stations::default()
            })
            .unwrap();
        let (_, first_ts) = store.read_latest_stations().unwrap();

        let stations = store.get_snapshot_by_timestamp(first_ts).unwrap();
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0].nb_bikes, 4);
    }

    #[test]
    fn station_metadata_upserts_across_snapshots() {
        let store = SqliteStorage::open_in_memory().unwrap();

        let mut s = station(1, 4, 1, 15);
        s.name = "Old Name".to_string();
        store
            .write_stations(&Stations {
                stations: vec![s],
                ..Stations::default()
            })
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(1100)); // force a distinct second
        let mut s2 = station(1, 2, 0, 17);
        s2.name = "New Name".to_string();
        store
            .write_stations(&Stations {
                stations: vec![s2],
                ..Stations::default()
            })
            .unwrap();

        let (latest, _) = store.read_latest_stations().unwrap();
        assert_eq!(latest[0].name, "New Name");
    }
}
