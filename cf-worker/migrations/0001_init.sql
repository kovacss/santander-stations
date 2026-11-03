-- Same schema reviewed and built for the local SQLite POC (see
-- ../../README.md "Local SQLite Storage"), applied here to D1 — D1 is
-- SQLite, so the DDL is unchanged.

-- Station metadata (id, name, coordinates) — rarely changes, so it's kept
-- separate from the time-series readings instead of repeated on every row.
CREATE TABLE IF NOT EXISTS stations (
    id      INTEGER PRIMARY KEY,
    name    TEXT NOT NULL,
    lat     REAL NOT NULL,
    long    REAL NOT NULL
);

-- One row per collector run.
CREATE TABLE IF NOT EXISTS snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    captured_at TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS idx_snapshots_captured_at ON snapshots (captured_at);

-- Per-station numbers for a given snapshot.
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

-- Precomputed per-snapshot totals — this is what makes /api/history fast.
CREATE TABLE IF NOT EXISTS snapshot_aggregates (
    snapshot_id       INTEGER PRIMARY KEY REFERENCES snapshots(id) ON DELETE CASCADE,
    captured_at       TEXT NOT NULL,
    total_bikes       INTEGER NOT NULL,
    total_ebikes      INTEGER NOT NULL,
    total_empty_docks INTEGER NOT NULL,
    station_count     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_snapshot_aggregates_captured_at ON snapshot_aggregates (captured_at);
