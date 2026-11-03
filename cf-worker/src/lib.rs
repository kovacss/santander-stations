//! Cloudflare Worker: serves the Santander Cycles map/API from D1, and runs
//! the data collector on a Cron Trigger.
//!
//! Two `#[event]` handlers in one Worker script:
//! - `fetch` — the web server (routes below), reading from D1.
//! - `scheduled` — the collector. Fires every 15 minutes (see
//!   `wrangler.toml`'s `[triggers]`), fetches the TFL feed and writes a
//!   snapshot to D1, except during an overnight pause (11pm-5am London
//!   time) to avoid piling up near-identical low-traffic snapshots.
//!
//! Response shapes match the tokio/axum server's HTTP API exactly (see
//! `../../src/web/handlers.rs`) — same JSON field names, same routes, same
//! "echo the requested timestamp back" behaviour for snapshot lookups — so
//! the existing frontend works unmodified against either.

mod db;
mod tfl;

use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use worker::*;

use db::HistoricalDataPoint;
use tfl::Station;

/// The map page. Static, so it's embedded in the binary rather than served
/// from D1 or R2.
const MAP_HTML: &str = include_str!("map.html");

/// London hours (local, DST-aware) during which the collector pauses, to
/// avoid piling up near-identical overnight snapshots. The Cron Trigger
/// itself fires continuously every 15 minutes (`wrangler.toml`); the pause
/// is enforced here rather than as a UTC cron window because a fixed UTC
/// window would drift by an hour across the BST/GMT transition.
const PAUSE_FROM_HOUR: u32 = 23; // 11pm
const PAUSE_UNTIL_HOUR: u32 = 5; // 5am

// ---------------------------------------------------------------------------
// Web server
// ---------------------------------------------------------------------------

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();

    Router::new()
        .get("/", |_req, _ctx| Response::from_html(MAP_HTML))
        .get_async("/api/stations", handle_stations)
        .get_async("/api/history", handle_history)
        .get_async("/api/history/snapshot", handle_history_snapshot)
        .run(req, env)
        .await
}

/// `GET /api/stations` — current station data.
async fn handle_stations(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let db = ctx.d1("DB")?;

    let (stations, timestamp) = match db::read_latest_stations(&db).await {
        Ok(result) => result,
        Err(err) => {
            // No stored snapshot yet - fall back to a live fetch, matching
            // the tokio server's behaviour.
            console_log!("No stored data ({err}), fetching live");
            let live = tfl::fetch_stations().await?;
            (live.stations, Utc::now())
        }
    };

    Response::from_json(&StationsResponse::new(timestamp, &stations))
}

/// `GET /api/history` — aggregate usage trends across all snapshots, newest
/// first.
async fn handle_history(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let db = ctx.d1("DB")?;
    let data_points = db::get_historical_data(&db).await?;

    Response::from_json(&HistoryResponse {
        data_points: data_points.iter().map(HistoryDataPointResponse::from).collect(),
    })
}

#[derive(Deserialize)]
struct SnapshotQuery {
    timestamp: Option<String>,
}

/// `GET /api/history/snapshot?timestamp=<rfc3339>` — station data for the
/// snapshot closest to a timestamp.
async fn handle_history_snapshot(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let query: SnapshotQuery = req.query()?;

    let Some(raw) = query.timestamp.filter(|v| !v.is_empty()) else {
        return Response::error("Missing timestamp parameter", 400);
    };

    let Ok(target) = DateTime::parse_from_rfc3339(&raw).map(|dt| dt.with_timezone(&Utc)) else {
        return Response::error("Invalid timestamp format", 400);
    };

    let db = ctx.d1("DB")?;
    let stations = db::get_snapshot_by_timestamp(&db, target).await?;

    // Echoes the requested timestamp, not the matched snapshot's actual
    // timestamp - matches the tokio server's contract.
    Response::from_json(&StationsResponse::new(target, &stations))
}

// ---------------------------------------------------------------------------
// Collector (Cron Trigger)
// ---------------------------------------------------------------------------

#[event(scheduled)]
pub async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    console_error_panic_hook::set_once();

    let now = Utc::now();
    if !collection_active_at(now) {
        console_log!("Within the overnight pause window ({now}), skipping fetch");
        return;
    }

    if let Err(err) = collect_once(&env).await {
        console_error!("Collector run failed: {err}");
    }
}

/// True outside the overnight pause window (11pm-5am London local time,
/// DST-aware).
fn collection_active_at(now_utc: DateTime<Utc>) -> bool {
    let london_hour = now_utc.with_timezone(&chrono_tz::Europe::London).hour();
    !(london_hour >= PAUSE_FROM_HOUR || london_hour < PAUSE_UNTIL_HOUR)
}

async fn collect_once(env: &Env) -> Result<()> {
    console_log!("Fetching station data...");

    let stations = tfl::fetch_stations().await?;
    let count = stations.stations.len();

    let db = env.d1("DB")?;
    let snapshot_id = db::write_stations(&db, &stations).await?;

    console_log!("Saved {count} stations to D1 (snapshot_id={snapshot_id})");
    Ok(())
}

// ---------------------------------------------------------------------------
// JSON response shapes — field names match the tokio/axum server exactly.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct StationResponse {
    id: i64,
    name: String,
    lat: f64,
    lng: f64,
    #[serde(rename = "nbBikes")]
    nb_bikes: i64,
    #[serde(rename = "nbStandardBikes")]
    nb_standard_bikes: i64,
    #[serde(rename = "nbEBikes")]
    nb_ebikes: i64,
    #[serde(rename = "nbEmptyDocks")]
    nb_empty_docks: i64,
    #[serde(rename = "nbDocks")]
    nb_docks: i64,
}

impl From<&Station> for StationResponse {
    fn from(s: &Station) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            lat: s.lat,
            lng: s.long,
            nb_bikes: s.nb_bikes,
            nb_standard_bikes: s.nb_standard_bikes,
            nb_ebikes: s.nb_ebikes,
            nb_empty_docks: s.nb_empty_docks,
            nb_docks: s.nb_docks,
        }
    }
}

#[derive(Serialize)]
struct StationsResponse {
    timestamp: String,
    stations: Vec<StationResponse>,
}

impl StationsResponse {
    fn new(timestamp: DateTime<Utc>, stations: &[Station]) -> Self {
        Self {
            timestamp: format_timestamp(timestamp),
            stations: stations.iter().map(StationResponse::from).collect(),
        }
    }
}

#[derive(Serialize)]
struct HistoryDataPointResponse {
    timestamp: String,
    #[serde(rename = "totalBikes")]
    total_bikes: i64,
    #[serde(rename = "totalEBikes")]
    total_ebikes: i64,
    #[serde(rename = "totalEmptyDocks")]
    total_empty_docks: i64,
    #[serde(rename = "stationCount")]
    station_count: i64,
}

impl From<&HistoricalDataPoint> for HistoryDataPointResponse {
    fn from(dp: &HistoricalDataPoint) -> Self {
        Self {
            timestamp: format_timestamp(dp.timestamp),
            total_bikes: dp.total_bikes,
            total_ebikes: dp.total_ebikes,
            total_empty_docks: dp.total_empty_docks,
            station_count: dp.station_count,
        }
    }
}

#[derive(Serialize)]
struct HistoryResponse {
    #[serde(rename = "dataPoints")]
    data_points: Vec<HistoryDataPointResponse>,
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
