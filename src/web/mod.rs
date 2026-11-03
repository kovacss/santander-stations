//! Web interface: routes, handlers and the embedded map page.

mod handlers;

pub use handlers::{
    router, AppState, HistoryDataPointResponse, HistoryResponse, StationResponse,
    StationsResponse,
};
