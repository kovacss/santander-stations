//! City Cycling: tracks Santander Cycle stations in London in real time and
//! serves an interactive web map.
//!
//! The crate exposes three binaries — `collector`, `collector-r2` and
//! `server` — built on top of these modules.

pub mod config;
pub mod storage;
pub mod tfl;
pub mod web;

use std::time::Duration;

/// Installs the log subscriber. Verbosity follows `RUST_LOG`, defaulting to
/// `info`.
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

/// Parses a `--interval` value such as `5m`, `90s` or `1h30m`.
///
/// `0` is accepted and means "run once", matching the Go collector.
pub fn parse_interval(raw: &str) -> Result<Duration, String> {
    let raw = raw.trim();

    if raw == "0" {
        return Ok(Duration::ZERO);
    }

    humantime::parse_duration(raw).map_err(|err| format!("invalid duration {raw:?}: {err}"))
}

/// Resolves once the process is asked to stop, yielding the signal's name.
#[cfg(unix)]
pub async fn shutdown_signal() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm =
        signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => "SIGINT",
        _ = sigterm.recv() => "SIGTERM",
    }
}

/// Resolves once the process is asked to stop, yielding the signal's name.
#[cfg(not(unix))]
pub async fn shutdown_signal() -> &'static str {
    let _ = tokio::signal::ctrl_c().await;
    "ctrl-c"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_intervals() {
        assert_eq!(parse_interval("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_interval("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_interval("0").unwrap(), Duration::ZERO);
        assert!(parse_interval("soon").is_err());
    }
}
