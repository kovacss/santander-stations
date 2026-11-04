//! TFL feed: JSON model and fetch, Worker-flavored.
//!
//! This hits TfL's official Unified API (`api.tfl.gov.uk/BikePoint`) rather
//! than the `tfl.gov.uk` website's XML syndication feed used by the tokio
//! implementation (`../src/tfl/`). That switch was forced, not stylistic:
//! the website's feed sits behind Cloudflare's Managed Challenge for
//! requests coming from the Workers network (a Cloudflare-Worker-calling-
//! another-Cloudflare-zone bot-management interaction - confirmed in
//! production via `wrangler tail`, see `../README.md`'s "TFL Fetch:
//! Cloudflare Managed Challenge" section). A Managed Challenge requires
//! executing JavaScript and passing a browser attestation, which no
//! server-side HTTP client - this Worker, curl, or anything else - can
//! satisfy, so no amount of header tuning could fix it. `api.tfl.gov.uk` is
//! also Cloudflare-fronted but, being a dedicated machine-consumption API
//! rather than the public website, doesn't appear to carry the same policy.
//!
//! The JSON shape is very different from the XML feed (an array of `Place`
//! objects, with bike/dock counts buried in a generic `additionalProperties`
//! key/value bag, string-typed even for numbers), but it maps onto the same
//! `Station`/`Stations` structs - the same two the D1 schema in `db.rs` and
//! the JSON API contract in `lib.rs` are written against - so nothing
//! downstream of `fetch_stations()` needed to change.
//!
//! This API does rate-limit (`429 Rate limit is exceeded`), seen for real on
//! the collector cron once traffic picked up. `fetch_stations()` retries a
//! 429, a 5xx, or a network-level send failure with exponential backoff -
//! see `fetch_body()` below. A 403 Managed Challenge (the reason this file
//! isn't hitting the XML feed at all) is deliberately *not* retried: a
//! second attempt wouldn't satisfy a JS challenge either, so retrying it
//! would only waste the Worker's time budget.

use std::time::Duration;

use serde::Deserialize;
use worker::{console_error, console_log, Delay, Fetch, Headers, Method, Request, RequestInit, Result};

/// TfL's Unified API: all BikePoints in one call.
const TFL_ENDPOINT: &str = "https://api.tfl.gov.uk/BikePoint/";

/// Retry up to this many times (so up to `MAX_RETRIES + 1` attempts total)
/// on a transient failure.
const MAX_RETRIES: u32 = 3;

/// Backoff base. TfL's own 429 response suggests "Try again in 2 seconds",
/// so retries wait 2s, 4s, 8s (doubling each time) rather than starting
/// from something arbitrary.
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);

/// Fetches and parses the current station data from the TFL BikePoint API.
pub async fn fetch_stations() -> Result<Stations> {
    let body = fetch_body().await?;

    let bike_points: Vec<BikePoint> = serde_json::from_str(&body)
        .map_err(|err| worker::Error::RustError(format!("failed to parse TFL JSON: {err}")))?;

    Ok(Stations {
        stations: bike_points.iter().map(Station::from).collect(),
    })
}

/// An error from a single fetch attempt, tagged with whether it's worth
/// retrying. Errors from `worker`'s own plumbing (setting a header, sending
/// the request) default to retryable - they're generally a transient
/// network blip rather than something a retry can't fix.
struct FetchError {
    retryable: bool,
    error: worker::Error,
}

impl From<worker::Error> for FetchError {
    fn from(error: worker::Error) -> Self {
        FetchError { retryable: true, error }
    }
}

/// A 429 (rate limited) or a 5xx (server-side issue) is worth retrying;
/// anything else - notably a 403 - is not.
fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// Fetches the raw response body, retrying transient failures (see
/// `is_retryable_status` and `FetchError`'s default) with exponential
/// backoff.
async fn fetch_body() -> Result<String> {
    let mut attempt = 0;

    loop {
        match fetch_once().await {
            Ok(body) => return Ok(body),
            Err(FetchError { retryable, error }) => {
                if !retryable || attempt >= MAX_RETRIES {
                    return Err(error);
                }
                attempt += 1;
                let delay = RETRY_BASE_DELAY * 2u32.pow(attempt - 1); // 2s, 4s, 8s
                console_error!(
                    "TFL fetch failed ({error}); retrying in {delay:?} (attempt {attempt}/{MAX_RETRIES})"
                );
                Delay::from(delay).await;
            }
        }
    }
}

/// A single fetch attempt.
async fn fetch_once() -> std::result::Result<String, FetchError> {
    let headers = Headers::new();
    headers.set("Accept", "application/json")?;
    headers.set(
        "User-Agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    )?;

    let mut init = RequestInit::new();
    init.with_method(Method::Get).with_headers(headers);

    let request = Request::new_with_init(TFL_ENDPOINT, &init)?;
    let mut response = Fetch::Request(request).send().await?; // network-level failure - retryable

    let status = response.status_code();
    console_log!("TFL response: status={status}");

    if status != 200 {
        let body = response.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(500).collect();
        console_error!("TFL response body (first 500 chars): {snippet}");

        return Err(FetchError {
            retryable: is_retryable_status(status),
            error: worker::Error::RustError(format!("unexpected status code from TFL: {status}")),
        });
    }

    Ok(response.text().await?)
}

/// A collection of stations, mirroring the tokio implementation's
/// `tfl::Stations` so `db.rs`/`lib.rs` need no changes.
#[derive(Debug, Clone, Default)]
pub struct Stations {
    pub stations: Vec<Station>,
}

/// A single Santander Cycles docking station. Same shape as the tokio
/// implementation's `tfl::Station` (and the D1 schema's `stations` /
/// `station_readings` columns) regardless of which upstream feed populates it.
#[derive(Debug, Clone, Default)]
pub struct Station {
    pub id: i64,
    pub name: String,
    pub lat: f64,
    pub long: f64,
    pub nb_bikes: i64,
    pub nb_standard_bikes: i64,
    pub nb_ebikes: i64,
    pub nb_empty_docks: i64,
    pub nb_docks: i64,
}

/// One TfL `Place` object as returned by `GET /BikePoint/`. Only the fields
/// this app uses; TfL's response has several more (`$type`, `url`,
/// `placeType`, `children`, ...) that `serde` ignores by default.
#[derive(Debug, Deserialize)]
struct BikePoint {
    /// e.g. `"BikePoints_1"` - the numeric suffix is this app's station id.
    id: String,
    #[serde(rename = "commonName")]
    common_name: String,
    lat: f64,
    lon: f64,
    #[serde(rename = "additionalProperties", default)]
    additional_properties: Vec<AdditionalProperty>,
}

/// A generic key/value entry in a `Place`'s property bag. TfL encodes every
/// value as a string regardless of its real type (e.g. `"NbBikes": "3"`,
/// `"Installed": "true"`).
#[derive(Debug, Deserialize)]
struct AdditionalProperty {
    key: String,
    value: String,
}

impl BikePoint {
    /// Looks up a named property and parses it as `i64`, defaulting to `0`
    /// if the key is missing or unparseable - TfL occasionally omits a
    /// property for a given station, and a strict lookup would otherwise
    /// fail the whole fetch over one station.
    fn property_i64(&self, key: &str) -> i64 {
        self.additional_properties
            .iter()
            .find(|p| p.key == key)
            .and_then(|p| p.value.trim().parse().ok())
            .unwrap_or(0)
    }
}

impl From<&BikePoint> for Station {
    fn from(bp: &BikePoint) -> Self {
        Station {
            id: parse_station_id(&bp.id),
            name: bp.common_name.clone(),
            lat: bp.lat,
            long: bp.lon,
            nb_bikes: bp.property_i64("NbBikes"),
            nb_standard_bikes: bp.property_i64("NbStandardBikes"),
            nb_ebikes: bp.property_i64("NbEBikes"),
            nb_empty_docks: bp.property_i64("NbEmptyDocks"),
            nb_docks: bp.property_i64("NbDocks"),
        }
    }
}

/// Extracts the numeric id from a TfL place id like `"BikePoints_1"`.
/// Defaults to `0` on anything unexpected - every real BikePoint id has
/// this exact `BikePoints_<n>` shape, so this should never actually trigger.
fn parse_station_id(id: &str) -> i64 {
    id.rsplit('_')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_statuses_are_429_and_5xx_only() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(599));
        assert!(!is_retryable_status(403)); // Managed Challenge - retrying never helps
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(200));
    }

    #[test]
    fn parses_a_bikepoint_into_a_station() {
        let bp = BikePoint {
            id: "BikePoints_1".to_string(),
            common_name: "River Street , Clerkenwell".to_string(),
            lat: 51.529163,
            lon: -0.10997,
            additional_properties: vec![
                AdditionalProperty { key: "NbBikes".to_string(), value: "3".to_string() },
                AdditionalProperty { key: "NbStandardBikes".to_string(), value: "3".to_string() },
                AdditionalProperty { key: "NbEBikes".to_string(), value: "0".to_string() },
                AdditionalProperty { key: "NbEmptyDocks".to_string(), value: "13".to_string() },
                AdditionalProperty { key: "NbDocks".to_string(), value: "19".to_string() },
            ],
        };

        let station = Station::from(&bp);
        assert_eq!(station.id, 1);
        assert_eq!(station.name, "River Street , Clerkenwell");
        assert!((station.lat - 51.529163).abs() < 1e-9);
        assert!((station.long - (-0.10997)).abs() < 1e-9);
        assert_eq!(station.nb_bikes, 3);
        assert_eq!(station.nb_standard_bikes, 3);
        assert_eq!(station.nb_ebikes, 0);
        assert_eq!(station.nb_empty_docks, 13);
        assert_eq!(station.nb_docks, 19);
    }

    #[test]
    fn missing_property_defaults_to_zero() {
        let bp = BikePoint {
            id: "BikePoints_42".to_string(),
            common_name: "Test Station".to_string(),
            lat: 0.0,
            lon: 0.0,
            additional_properties: vec![],
        };

        let station = Station::from(&bp);
        assert_eq!(station.id, 42);
        assert_eq!(station.nb_bikes, 0);
    }

    #[test]
    fn parses_full_api_response_shape() {
        // A trimmed real response (extra TfL fields included, to prove
        // they're correctly ignored rather than causing a parse error).
        let json = r#"[{
            "$type": "Tfl.Api.Presentation.Entities.Place, Tfl.Api.Presentation.Entities",
            "id": "BikePoints_1",
            "url": "/Place/BikePoints_1",
            "commonName": "River Street , Clerkenwell",
            "placeType": "BikePoint",
            "additionalProperties": [
                {"category": "Description", "key": "NbBikes", "sourceSystemKey": "BikePoints", "value": "3", "modified": "2026-11-03T20:18:01.767Z"},
                {"category": "Description", "key": "NbEmptyDocks", "sourceSystemKey": "BikePoints", "value": "13", "modified": "2026-11-03T20:18:01.767Z"},
                {"category": "Description", "key": "NbDocks", "sourceSystemKey": "BikePoints", "value": "19", "modified": "2026-11-03T20:18:01.767Z"},
                {"category": "Description", "key": "NbStandardBikes", "sourceSystemKey": "BikePoints", "value": "3", "modified": "2026-11-03T20:18:01.767Z"},
                {"category": "Description", "key": "NbEBikes", "sourceSystemKey": "BikePoints", "value": "0", "modified": "2026-11-03T20:18:01.767Z"}
            ],
            "children": [],
            "lat": 51.529163,
            "lon": -0.10997
        }]"#;

        let bike_points: Vec<BikePoint> = serde_json::from_str(json).unwrap();
        assert_eq!(bike_points.len(), 1);

        let station = Station::from(&bike_points[0]);
        assert_eq!(station.id, 1);
        assert_eq!(station.nb_bikes, 3);
        assert_eq!(station.nb_docks, 19);
    }
}
