//! TFL API client and data model.

mod client;
mod models;

pub use client::{Client, DEFAULT_ENDPOINT, DEFAULT_TIMEOUT};
pub use models::{Station, Stations};
