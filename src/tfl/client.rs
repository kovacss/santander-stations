//! HTTP client for the TFL Santander Cycles feed.

use std::time::Duration;

use anyhow::{bail, Context, Result};

use super::models::Stations;

/// The TFL Santander Cycles live update feed.
pub const DEFAULT_ENDPOINT: &str =
    "https://tfl.gov.uk/tfl/syndication/feeds/cycle-hire/livecyclehireupdates.xml";

/// Default timeout for HTTP requests.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Client fetches station data from the TFL API.
#[derive(Debug, Clone)]
pub struct Client {
    endpoint: String,
    http: reqwest::Client,
}

impl Client {
    /// Creates a new TFL client with default settings.
    pub fn new() -> Self {
        Self::with_endpoint(DEFAULT_ENDPOINT)
    }

    /// Creates a new TFL client pointed at a custom endpoint.
    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent("city-cycling/1.0")
            .build()
            .expect("failed to build HTTP client");

        Self {
            endpoint: endpoint.into(),
            http,
        }
    }

    /// Retrieves the current station data from the TFL API.
    pub async fn fetch_stations(&self) -> Result<Stations> {
        let resp = self
            .http
            .get(&self.endpoint)
            .send()
            .await
            .context("failed to fetch stations")?;

        if !resp.status().is_success() {
            bail!("unexpected status code: {}", resp.status().as_u16());
        }

        let body = resp
            .text()
            .await
            .context("failed to read response body")?;

        quick_xml::de::from_str(&body).context("failed to parse XML")
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}
