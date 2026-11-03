//! D1 queries.
//!
//! Same schema and the same reasoning as the local SQLite POC (see
//! `../../README.md`, "Local SQLite Storage"): `snapshot_aggregates` is
//! precomputed at write time so `/api/history` is a flat indexed read, not a
//! per-request scan. D1 is SQLite, so the schema and the SQL are unchanged —
//! only how statements get bound and executed differs, through the Workers
//! D1 binding (`worker::d1`) instead of `rusqlite`.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use worker::wasm_bindgen::JsValue;
use worker::*;

use crate::tfl::{Station, Stations};

/// A snapshot in time with aggregate statistics.
#[derive(Debug, Clone)]
pub struct HistoricalDataPoint {
    pub timestamp: DateTime<Utc>,
    pub total_bikes: i64,
    pub total_ebikes: i64,
    pub total_empty_docks: i64,
    pub station_count: i64,
}

/// Writes a full snapshot — the `snapshots` row, upserted `stations`
/// metadata, every `station_readings` row, and the precomputed
/// `snapshot_aggregates` row — and returns the new snapshot's id.
///
/// The snapshot id is needed to build every other statement, so it's
/// inserted (with `RETURNING id`) before the rest; everything else goes in
/// one `batch()` call. D1 counts a whole batch as a single query against
/// the per-invocation query limit, and the per-statement limit (100 bound
/// parameters) is what rules out a single multi-row `INSERT ... VALUES
/// (...), (...), ...` for ~800 stations — one statement per row, binding
/// well under the limit, sidesteps that.
pub async fn write_stations(db: &D1Database, stations: &Stations) -> Result<i64> {
    let timestamp = Utc::now();
    let captured_at = timestamp.to_rfc3339_opts(SecondsFormat::Secs, true);

    let snapshot_id: i64 = db
        .prepare("INSERT INTO snapshots (captured_at) VALUES (?1) RETURNING id")
        .bind(&[d1_text(&captured_at)])?
        .first(Some("id"))
        .await?
        .ok_or_else(|| Error::RustError("insert into snapshots returned no id".into()))?;

    let mut batch: Vec<D1PreparedStatement> = Vec::with_capacity(stations.stations.len() * 2 + 1);

    const UPSERT_STATION_SQL: &str = "INSERT INTO stations (id, name, lat, long) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name, lat = excluded.lat, long = excluded.long";
    const INSERT_READING_SQL: &str = "INSERT INTO station_readings
             (snapshot_id, station_id, nb_bikes, nb_standard_bikes, nb_ebikes, nb_empty_docks, nb_docks)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

    for s in &stations.stations {
        batch.push(
            db.prepare(UPSERT_STATION_SQL)
                .bind(&[d1_int(s.id), d1_text(&s.name), d1_real(s.lat), d1_real(s.long)])?,
        );
        batch.push(
            db.prepare(INSERT_READING_SQL).bind(&[
                d1_int(snapshot_id),
                d1_int(s.id),
                d1_int(s.nb_bikes),
                d1_int(s.nb_standard_bikes),
                d1_int(s.nb_ebikes),
                d1_int(s.nb_empty_docks),
                d1_int(s.nb_docks),
            ])?,
        );
    }

    let total_bikes: i64 = stations.stations.iter().map(|s| s.nb_bikes).sum();
    let total_ebikes: i64 = stations.stations.iter().map(|s| s.nb_ebikes).sum();
    let total_empty_docks: i64 = stations.stations.iter().map(|s| s.nb_empty_docks).sum();
    let station_count = stations.stations.len() as i64;

    batch.push(
        db.prepare(
            "INSERT INTO snapshot_aggregates
                 (snapshot_id, captured_at, total_bikes, total_ebikes, total_empty_docks, station_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&[
            d1_int(snapshot_id),
            d1_text(&captured_at),
            d1_int(total_bikes),
            d1_int(total_ebikes),
            d1_int(total_empty_docks),
            d1_int(station_count),
        ])?,
    );

    db.batch(batch).await?;

    Ok(snapshot_id)
}

/// Reads the most recent snapshot's stations.
pub async fn read_latest_stations(db: &D1Database) -> Result<(Vec<Station>, DateTime<Utc>)> {
    #[derive(Deserialize)]
    struct Row {
        id: i64,
        captured_at: String,
    }

    let latest: Option<Row> = db
        .prepare("SELECT id, captured_at FROM snapshots ORDER BY captured_at DESC LIMIT 1")
        .first(None)
        .await?;

    let Some(latest) = latest else {
        return Err(Error::RustError("no snapshots found in D1 database".into()));
    };

    let stations = stations_for_snapshot(db, latest.id).await?;
    Ok((stations, parse_rfc3339(&latest.captured_at)?))
}

/// Returns aggregate statistics for every snapshot, newest first.
///
/// This is the point of the schema: a flat read of `snapshot_aggregates`,
/// no joins and no per-request summation.
pub async fn get_historical_data(db: &D1Database) -> Result<Vec<HistoricalDataPoint>> {
    #[derive(Deserialize)]
    struct Row {
        captured_at: String,
        total_bikes: i64,
        total_ebikes: i64,
        total_empty_docks: i64,
        station_count: i64,
    }

    let rows: Vec<Row> = db
        .prepare(
            "SELECT captured_at, total_bikes, total_ebikes, total_empty_docks, station_count
             FROM snapshot_aggregates
             ORDER BY captured_at DESC",
        )
        .all()
        .await?
        .results()?;

    rows.into_iter()
        .map(|r| {
            Ok(HistoricalDataPoint {
                timestamp: parse_rfc3339(&r.captured_at)?,
                total_bikes: r.total_bikes,
                total_ebikes: r.total_ebikes,
                total_empty_docks: r.total_empty_docks,
                station_count: r.station_count,
            })
        })
        .collect()
}

/// Returns station data for the snapshot whose `captured_at` is closest to
/// `target`.
pub async fn get_snapshot_by_timestamp(db: &D1Database, target: DateTime<Utc>) -> Result<Vec<Station>> {
    #[derive(Deserialize)]
    struct Row {
        id: i64,
    }

    // Bound as a REAL rather than D1Type::Integer (i32 only) — an i32 epoch
    // second count overflows in 2038, and f64 represents it exactly for any
    // realistic date.
    let closest: Option<Row> = db
        .prepare(
            "SELECT id FROM snapshots
             ORDER BY ABS(strftime('%s', captured_at) - ?1) ASC
             LIMIT 1",
        )
        .bind(&[d1_real(target.timestamp() as f64)])?
        .first(None)
        .await?;

    let Some(closest) = closest else {
        return Err(Error::RustError("no snapshots available".into()));
    };

    stations_for_snapshot(db, closest.id).await
}

/// Joins `station_readings` back onto `stations` for one snapshot.
async fn stations_for_snapshot(db: &D1Database, snapshot_id: i64) -> Result<Vec<Station>> {
    #[derive(Deserialize)]
    struct Row {
        id: i64,
        name: String,
        lat: f64,
        long: f64,
        nb_bikes: i64,
        nb_standard_bikes: i64,
        nb_ebikes: i64,
        nb_empty_docks: i64,
        nb_docks: i64,
    }

    let rows: Vec<Row> = db
        .prepare(
            "SELECT s.id, s.name, s.lat, s.long,
                    r.nb_bikes, r.nb_standard_bikes, r.nb_ebikes, r.nb_empty_docks, r.nb_docks
             FROM station_readings r
             JOIN stations s ON s.id = r.station_id
             WHERE r.snapshot_id = ?1
             ORDER BY s.id",
        )
        .bind(&[d1_int(snapshot_id)])?
        .all()
        .await?
        .results()?;

    Ok(rows
        .into_iter()
        .map(|r| Station {
            id: r.id,
            name: r.name,
            lat: r.lat,
            long: r.long,
            nb_bikes: r.nb_bikes,
            nb_standard_bikes: r.nb_standard_bikes,
            nb_ebikes: r.nb_ebikes,
            nb_empty_docks: r.nb_empty_docks,
            nb_docks: r.nb_docks,
        })
        .collect())
}

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| Error::RustError(format!("invalid timestamp {raw:?} in database: {err}")))
}

fn d1_int(v: i64) -> JsValue {
    JsValue::from(&D1Type::Integer(v as i32))
}

fn d1_real(v: f64) -> JsValue {
    JsValue::from(&D1Type::Real(v))
}

fn d1_text(v: &str) -> JsValue {
    JsValue::from(&D1Type::Text(v))
}
