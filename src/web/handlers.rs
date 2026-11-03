//! HTTP handlers for the web interface.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::storage::{HistoricalDataPoint, Store, StoreError};
use crate::tfl::{self, Station};

/// How long to cache the aggregate historical data.
const HISTORY_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

/// The map page. It takes no server-side data, so it ships as a static asset
/// baked into the binary rather than going through a template engine.
const MAP_HTML: &str = include_str!("templates/map.html");

/// JSON response format for a single station.
#[derive(Debug, Serialize)]
pub struct StationResponse {
    pub id: i64,
    pub name: String,
    pub lat: f64,
    pub lng: f64,
    #[serde(rename = "nbBikes")]
    pub nb_bikes: i64,
    #[serde(rename = "nbStandardBikes")]
    pub nb_standard_bikes: i64,
    #[serde(rename = "nbEBikes")]
    pub nb_ebikes: i64,
    #[serde(rename = "nbEmptyDocks")]
    pub nb_empty_docks: i64,
    #[serde(rename = "nbDocks")]
    pub nb_docks: i64,
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

/// JSON response for the stations API.
#[derive(Debug, Serialize)]
pub struct StationsResponse {
    pub timestamp: String,
    pub stations: Vec<StationResponse>,
}

impl StationsResponse {
    fn new(timestamp: DateTime<Utc>, stations: &[Station]) -> Self {
        Self {
            timestamp: format_timestamp(timestamp),
            stations: stations.iter().map(StationResponse::from).collect(),
        }
    }
}

/// Historical aggregate data at a point in time.
#[derive(Debug, Serialize)]
pub struct HistoryDataPointResponse {
    pub timestamp: String,
    #[serde(rename = "totalBikes")]
    pub total_bikes: i64,
    #[serde(rename = "totalEBikes")]
    pub total_ebikes: i64,
    #[serde(rename = "totalEmptyDocks")]
    pub total_empty_docks: i64,
    #[serde(rename = "stationCount")]
    pub station_count: i64,
}

/// JSON response for the history API.
#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    #[serde(rename = "dataPoints")]
    pub data_points: Vec<HistoryDataPointResponse>,
}

/// Shared state for the HTTP handlers.
pub struct AppState {
    store: Store,
    tfl: tfl::Client,
    /// Aggregate history, refreshed on a TTL.
    history_cache: RwLock<Option<(Instant, Arc<Vec<HistoricalDataPoint>>)>>,
    /// Snapshots keyed by normalized timestamp. Snapshots are immutable once
    /// written, so these entries never need to expire.
    snapshot_cache: RwLock<HashMap<String, Arc<Vec<Station>>>>,
}

impl AppState {
    pub fn new(store: Store, tfl: tfl::Client) -> Self {
        Self {
            store,
            tfl,
            history_cache: RwLock::new(None),
            snapshot_cache: RwLock::new(HashMap::new()),
        }
    }
}

/// Builds the application router with all routes registered.
pub fn router(store: Store, tfl: tfl::Client) -> Router {
    let state = Arc::new(AppState::new(store, tfl));

    Router::new()
        .route("/", get(handle_map))
        .route("/api/stations", get(handle_stations))
        .route("/api/history", get(handle_history))
        .route("/api/history/snapshot", get(handle_history_snapshot))
        .layer(axum::middleware::from_fn(log_requests))
        .with_state(state)
}

/// Logs each request with its status code and duration.
async fn log_requests(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let start = Instant::now();

    let response = next.run(req).await;

    tracing::info!(
        "[HTTP] {method} {path} - {} ({:.1?})",
        response.status().as_u16(),
        start.elapsed()
    );

    response
}

/// An error rendered as a plain-text HTTP response, mirroring Go's
/// `http.Error`.
struct ApiError(StatusCode, String);

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self(status, message.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, format!("{}\n", self.1)).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        match err {
            // The active backend cannot serve this; not a failure of the request.
            StoreError::Unsupported(message) => {
                ApiError::new(StatusCode::NOT_IMPLEMENTED, message)
            }
            StoreError::Other(err) => {
                tracing::error!("Storage error: {err:#}");
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to fetch station data",
                )
            }
        }
    }
}

/// Serves the main map page.
async fn handle_map() -> Html<&'static str> {
    Html(MAP_HTML)
}

/// Serves the stations API endpoint.
async fn handle_stations(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    // Prefer stored data, and fall back to the live API when there is none.
    let (stations, timestamp) = match state.store.read_latest_stations().await {
        Ok(latest) => latest,
        Err(err) => {
            tracing::info!("No stored data, fetching live: {err}");

            let live = state.tfl.fetch_stations().await.map_err(|err| {
                tracing::error!("Live fetch failed: {err:#}");
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to fetch station data",
                )
            })?;

            // Go stamps the fallback with a zero time, which renders as year
            // 0001 in the UI; the data really is from now.
            (live.stations, Utc::now())
        }
    };

    Ok(Json(StationsResponse::new(timestamp, &stations)))
}

/// Serves historical usage data, cached for [`HISTORY_CACHE_TTL`].
async fn handle_history(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some((cached_at, data_points)) = state.history_cache.read().await.as_ref() {
        if cached_at.elapsed() < HISTORY_CACHE_TTL {
            tracing::info!("History cache hit ({} data points)", data_points.len());
            return Ok(history_response(data_points));
        }
    }

    let data_points = Arc::new(state.store.get_historical_data().await?);

    *state.history_cache.write().await = Some((Instant::now(), Arc::clone(&data_points)));
    tracing::info!("History cache updated ({} data points)", data_points.len());

    Ok(history_response(&data_points))
}

/// Builds the history response.
fn history_response(data_points: &[HistoricalDataPoint]) -> Response {
    let body = HistoryResponse {
        data_points: data_points
            .iter()
            .map(|dp| HistoryDataPointResponse {
                timestamp: format_timestamp(dp.timestamp),
                total_bikes: dp.total_bikes,
                total_ebikes: dp.total_ebikes,
                total_empty_docks: dp.total_empty_docks,
                station_count: dp.station_count,
            })
            .collect(),
    };

    ([("cache-control", "public, max-age=3600")], Json(body)).into_response()
}

#[derive(Debug, Deserialize)]
struct SnapshotQuery {
    timestamp: Option<String>,
}

/// Serves station data for a specific timestamp from historical snapshots.
async fn handle_history_snapshot(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SnapshotQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let raw = query
        .timestamp
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(StatusCode::BAD_REQUEST, "Missing timestamp parameter")
        })?;

    let target = DateTime::parse_from_rfc3339(&raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "Invalid timestamp format"))?;

    // Snapshots are immutable, so the normalized timestamp is a stable key.
    let cache_key = target.to_rfc3339();

    if let Some(stations) = state.snapshot_cache.read().await.get(&cache_key) {
        tracing::info!(
            "Snapshot cache hit for {cache_key} ({} stations)",
            stations.len()
        );
        return Ok(snapshot_response(target, stations));
    }

    let stations = Arc::new(state.store.get_snapshot_by_timestamp(target).await?);

    state
        .snapshot_cache
        .write()
        .await
        .insert(cache_key.clone(), Arc::clone(&stations));
    tracing::info!(
        "Snapshot cache updated for {cache_key} ({} stations)",
        stations.len()
    );

    Ok(snapshot_response(target, &stations))
}

/// Builds the snapshot response.
fn snapshot_response(timestamp: DateTime<Utc>, stations: &[Station]) -> Response {
    (
        // Snapshots never change, so clients can hold them for a week.
        [("cache-control", "public, max-age=604800, immutable")],
        Json(StationsResponse::new(timestamp, stations)),
    )
        .into_response()
}

/// Formats a timestamp the way the API contract (and the frontend) expects.
fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
