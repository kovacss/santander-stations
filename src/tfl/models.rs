//! XML parsing structures for the TFL live cycle hire feed.

use serde::{Deserialize, Deserializer};

/// Stations represents the root XML element containing all bike stations.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Stations {
    #[serde(rename = "@lastUpdate", default, deserialize_with = "lenient_i64")]
    pub last_update: i64,
    #[serde(rename = "@version", default)]
    pub version: String,
    #[serde(rename = "station", default)]
    pub stations: Vec<Station>,
}

/// Station represents a single Santander Cycles docking station.
///
/// Numeric and boolean fields go through lenient deserializers: the feed
/// occasionally emits empty elements (`<nbEBikes/>`) for older stations, which
/// a strict parser would reject outright.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Station {
    #[serde(default, deserialize_with = "lenient_i64")]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "terminalName", default)]
    pub terminal_name: String,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub lat: f64,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub long: f64,
    #[serde(default, deserialize_with = "lenient_bool")]
    pub installed: bool,
    #[serde(default, deserialize_with = "lenient_bool")]
    pub locked: bool,
    #[serde(rename = "installDate", default)]
    pub install_date: String,
    #[serde(rename = "removalDate", default)]
    pub removal_date: String,
    #[serde(default, deserialize_with = "lenient_bool")]
    pub temporary: bool,
    #[serde(rename = "nbBikes", default, deserialize_with = "lenient_i64")]
    pub nb_bikes: i64,
    #[serde(rename = "nbStandardBikes", default, deserialize_with = "lenient_i64")]
    pub nb_standard_bikes: i64,
    #[serde(rename = "nbEBikes", default, deserialize_with = "lenient_i64")]
    pub nb_ebikes: i64,
    #[serde(rename = "nbEmptyDocks", default, deserialize_with = "lenient_i64")]
    pub nb_empty_docks: i64,
    #[serde(rename = "nbDocks", default, deserialize_with = "lenient_i64")]
    pub nb_docks: i64,
}

fn text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

fn lenient_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(text(d)?.trim().parse().unwrap_or(0))
}

fn lenient_f64<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(text(d)?.trim().parse().unwrap_or(0.0))
}

fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(matches!(
        text(d)?.trim().to_ascii_lowercase().as_str(),
        "true" | "1"
    ))
}
