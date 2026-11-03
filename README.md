# City Cycling

Tracks Santander Cycle stations in London in real time and serves an
interactive web-based map. Originally built in Go, then ported to Rust; the
Go implementation has since been removed from this repository (it's still
in the git history prior to the port) and Rust is now the only
implementation here.

Two deployable pieces live in this repo:

- **This crate** (`src/`, binaries below): a `tokio`/`axum` server plus
  collector CLIs, storing data in Cloudflare R2 or local TSV/SQLite files.
  Deploy anywhere that runs a long-lived process (Railway, a VM, etc.).
- **[`cf-worker/`](cf-worker/)**: a Cloudflare Worker — the actual
  Cloudflare-native deployment target, using D1 (SQLite) instead of R2, with
  the collector running as a Cron Trigger. See its own
  [README](cf-worker/README.md) for deployment steps.

## Features

- **Real-time Data Collection**: Fetches current station data from the Transport for London (TFL) API
- **Historical Tracking**: Stores timestamped snapshots of station data in TSV format
- **Interactive Map**: Web-based UI displaying all stations on a map with real-time availability
- **Color-Coded Status**: Green (bikes available), yellow (low availability), red (no bikes)
- **Station Details**: Click any marker to view bike counts and dock information
- **Historical Replay**: Scrub or play back station availability over every stored snapshot
- **SQLite Storage (experimental)**: Local SQLite backend with precomputed history aggregates — a POC for a future Cloudflare D1 backend, ~200x faster than R2 for history queries at scale

## Project Structure

```
city-cycling/
├── Cargo.toml
├── src/
│   ├── lib.rs                  # Shared setup: logging, interval parsing, shutdown
│   ├── bin/
│   │   ├── collector.rs        # Local data collection CLI
│   │   ├── collector_r2.rs     # R2 data collection CLI
│   │   ├── collector_sqlite.rs # SQLite data collection CLI (POC for Cloudflare D1)
│   │   └── server.rs           # Web server
│   ├── config.rs               # R2 configuration from env/.env
│   ├── tfl/
│   │   ├── client.rs           # TFL API HTTP client
│   │   └── models.rs           # XML parsing structures
│   ├── storage/
│   │   ├── mod.rs              # Store enum + shared error type
│   │   ├── tsvfmt.rs           # TSV render/parse, shared by both backends
│   │   ├── tsv.rs              # Local filesystem operations
│   │   ├── r2.rs               # Cloudflare R2 operations
│   │   └── sqlite.rs           # SQLite operations (POC for Cloudflare D1)
│   └── web/
│       ├── handlers.rs         # Routes and HTTP handlers
│       └── templates/map.html  # Map UI (embedded in the binary)
├── cf-worker/                   # Cloudflare Worker (fetch + D1 + cron) — see cf-worker/README.md
└── data/                        # TSV/SQLite data storage (auto-created)
```

## Installation

Requires Rust 1.75 or higher (install via [rustup](https://rustup.rs)).

```bash
cargo build --release
```

Binaries land in `target/release/{collector,collector-r2,collector-sqlite,server}`.

## Usage

### Local Data Collector

Fetch and store station data on the local filesystem:

```bash
# One-shot fetch
cargo run --bin collector -- --once

# Continuous mode (default 5-minute interval)
cargo run --bin collector

# Custom interval (e.g. every 10 minutes)
cargo run --bin collector -- --interval 10m

# Custom output directory
cargo run --bin collector -- --data-dir ./data
```

The collector creates timestamped TSV files in the `data/` directory.

### Cloudflare R2 Data Collector

**Local development (using a `.env` file):**

1. Copy `.env.example` to `.env`:
   ```bash
   cp .env.example .env
   ```

2. Fill in your Cloudflare R2 credentials:
   ```
   S3_ACCESS_KEY_ID=your_access_key_id
   S3_SECRET_ACCESS_KEY=your_secret_access_key
   S3_ENDPOINT=https://xxxxx.r2.cloudflarestorage.com
   S3_BUCKET_NAME=your-bucket-name
   S3_PREFIX=snapshots/
   ```

3. Run the collector:
   ```bash
   # One-shot fetch
   cargo run --bin collector-r2 -- --once

   # Continuous mode with custom interval
   cargo run --bin collector-r2 -- --interval 5m
   ```

**Production (using environment variables):**

Railway sets the environment variables, so the command is just:

```bash
./collector-r2 --once
```

The collector writes the same TSV columns as the Go implementation:

| Column | Meaning |
| --- | --- |
| `timestamp` | ISO 8601 timestamp of the fetch |
| `id` | Station ID |
| `name` | Station name |
| `lat` | Latitude coordinate |
| `long` | Longitude coordinate |
| `nb_bikes` | Total bikes available |
| `nb_standard_bikes` | Standard bikes |
| `nb_ebikes` | E-bikes |
| `nb_empty_docks` | Empty docks |
| `nb_docks` | Total docks |

### SQLite Data Collector (POC for Cloudflare D1)

Fetch and store station data into a local SQLite file instead of R2. This
exists to fix a real performance problem with the R2 backend: `/api/history`
there has to download and parse every historical snapshot file to sum a
handful of columns, which gets slow as the bucket grows (measured at 51s cold
against a 3,096-snapshot bucket). The SQLite backend precomputes those sums at
write time, so the same query becomes a flat indexed read (~0.25s at the same
scale). See [Local SQLite Storage](#local-sqlite-storage-poc-for-cloudflare-d1)
below for the schema.

This backend currently has no Go equivalent and is Rust-only.

```bash
# One-shot fetch into data/city-cycling.db (default path)
cargo run --bin collector-sqlite -- --once

# Continuous mode (default 5-minute interval)
cargo run --bin collector-sqlite

# Custom database path / interval
cargo run --bin collector-sqlite -- --db ./data/city-cycling.db --interval 10m
```

The database file (and any parent directory) is created automatically if it
doesn't exist.

### Web Server

**Local development (reads from local TSV files):**

```bash
cargo run --bin server -- --r2 false
```

**Local development (reads from a local SQLite file):**

```bash
cargo run --bin server -- --sqlite ./data/city-cycling.db
```

`--sqlite` takes priority over `--r2`/`USE_R2` when both are set. It can also
be set via the `SQLITE_DB` environment variable.

**Production (reads from Cloudflare R2):**

```bash
USE_R2=true cargo run --bin server
```

Or with everything in the environment:

```bash
export S3_ACCESS_KEY_ID=your_key
export S3_SECRET_ACCESS_KEY=your_secret
export S3_ENDPOINT=https://xxxxx.r2.cloudflarestorage.com
export S3_BUCKET_NAME=your-bucket
export S3_REGION=auto
export USE_R2=true

cargo run --bin server
```

The server listens on `http://localhost:8080` (override with `--port` or the
`PORT` environment variable) and serves the interactive map of all ~800
Santander Cycle stations from the configured storage backend.

Log verbosity follows `RUST_LOG` and defaults to `info`:

```bash
RUST_LOG=debug cargo run --bin server
```

## API Endpoints

- `GET /` — Serves the interactive map interface
- `GET /api/stations` — Current station data as JSON
- `GET /api/history` — Aggregate usage trends across all snapshots, newest first (R2 or SQLite backend only)
- `GET /api/history/snapshot?timestamp=<rfc3339>` — Station data for the snapshot closest to a timestamp (R2 or SQLite backend only)

Responses are byte-for-byte compatible with the Go server, so the same frontend
works against either.

### History API Response Format

```json
{
  "dataPoints": [
    {
      "timestamp": "2026-02-05T16:45:00Z",
      "totalBikes": 5432,
      "totalEBikes": 892,
      "totalEmptyDocks": 4120,
      "stationCount": 800
    }
  ]
}
```

## Data Format

Station data is stored as tab-separated values for easy analysis and historical
tracking:

```
timestamp	id	name	lat	long	nb_bikes	nb_standard_bikes	nb_ebikes	nb_empty_docks	nb_docks
2026-02-05T14:47:14Z	1	River Street , Clerkenwell	51.529163	-0.109971	0	0	0	10	19
2026-02-05T14:47:14Z	2	Phillimore Gardens, Kensington	51.499607	-0.197574	3	1	2	29	37
```

## Local SQLite Storage (POC for Cloudflare D1)

An alternative to the R2 backend, added specifically to fix slow history
queries. R2 has no query capability of its own: `/api/history` has to list
every snapshot object and download+parse each one just to sum four columns,
which is O(snapshots) network round trips. The SQLite backend precomputes
those sums once, at write time, into a small aggregates table:

```sql
-- Station metadata (id, name, coordinates) — rarely changes, so it's kept
-- separate from the time-series readings instead of repeated on every row.
CREATE TABLE stations (
    id      INTEGER PRIMARY KEY,   -- TFL station id
    name    TEXT NOT NULL,
    lat     REAL NOT NULL,
    long    REAL NOT NULL
);

-- One row per collector run.
CREATE TABLE snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    captured_at TEXT NOT NULL UNIQUE   -- RFC3339 UTC
);

-- Per-station numbers for a given snapshot.
CREATE TABLE station_readings (
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
CREATE TABLE snapshot_aggregates (
    snapshot_id       INTEGER PRIMARY KEY REFERENCES snapshots(id) ON DELETE CASCADE,
    captured_at       TEXT NOT NULL,
    total_bikes       INTEGER NOT NULL,
    total_ebikes      INTEGER NOT NULL,
    total_empty_docks INTEGER NOT NULL,
    station_count     INTEGER NOT NULL
);
```

`/api/history` then becomes a single flat `SELECT ... FROM snapshot_aggregates
ORDER BY captured_at DESC` — no joins, no per-request aggregation.

**Measured against the real R2 bucket's scale** (3,096 snapshots × 798
stations, synthetic data seeded into SQLite for an apples-to-apples
comparison):

| Backend | `/api/history` latency |
| --- | --- |
| R2 (cold) | 51.3s |
| SQLite | 0.25s |

Notes:

- This is plain, portable SQLite DDL — no extensions, no `WITHOUT ROWID` — so
  moving it to Cloudflare D1 later should mean swapping the connection, not
  the SQL. D1 is reachable either from inside a Cloudflare Worker via its
  native binding (a different runtime/deployment model from this project's
  current Railway-hosted `axum`/`tokio` binaries), or via D1's HTTP API (a
  plain REST call with an API token, callable from the current binaries
  as-is). Which path to take is still an open decision.
- `snapshots.captured_at` has a `UNIQUE` constraint at second resolution, so
  two writes within the same second collide. Not an issue at any real
  collector interval (minutes), but worth knowing if you're scripting rapid
  `--once` runs for testing.
- Implemented with `rusqlite` (the `bundled` feature compiles SQLite in, so
  nothing needs to be installed on the system) behind a `Mutex<Connection>`,
  wrapped in `spawn_blocking` — adequate for a POC; a connection pool
  (`r2d2_sqlite`) would be the next step for concurrent write-heavy use.
- This backend is Rust-only; there's no Go equivalent.

## Technical Details

| Concern | Crate |
| --- | --- |
| Async runtime | `tokio` |
| Web framework | `axum` |
| HTTP client | `reqwest` (rustls) |
| XML parsing | `quick-xml` + `serde` |
| Cloudflare R2 | `aws-sdk-s3` |
| Local SQLite (D1 POC) | `rusqlite` (bundled) |
| CLI flags | `clap` |
| Time | `chrono` |
| Logging | `tracing` |
| Errors | `anyhow` + `thiserror` |

Mapping is Leaflet.js over OpenStreetMap tiles; the map page is embedded in the
binary with `include_str!`, so the server ships as a single self-contained
executable.

## Tests

```bash
cargo test
```

Covers the TSV render/parse round trip (including tab escaping in station
names), snapshot filename handling, interval parsing, and the SQLite backend's
write/read round trip, newest-first history ordering, nearest-timestamp
lookup, and station-metadata upserts across snapshots.

## Local Development Workflow

1. Start the collector in the background:
   ```bash
   cargo run --bin collector &
   ```

2. Start the web server in another terminal:
   ```bash
   cargo run --bin server -- --r2 false
   ```

3. Open `http://localhost:8080`

4. The map updates as new data is collected

## Deployment (Railway + Cloudflare R2)

Railway's Nixpacks builder detects a Cargo project automatically; no Dockerfile
is required.

**Web server service**
- Build command: `cargo build --release --bin server`
- Start command: `./target/release/server`
- Environment: `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`, `S3_ENDPOINT`, `S3_BUCKET_NAME`, `S3_REGION=auto`, `USE_R2=true`, `PORT=8080`

**Collector service (cron)**
- Build command: `cargo build --release --bin collector-r2`
- Start command: `./target/release/collector-r2 --once`
- Cron schedule: `*/5 * * * *`
- Environment: the same `S3_*` variables

## Design Notes (from the Original Go Port)

This started as a line-for-line Rust port of a Go implementation. The Go
code has since been removed from this repo (it's still recoverable from git
history before the removal commit), but the design decisions made while
porting it are still relevant, so they're kept here. Behaviour and wire
formats were identical to the Go version for the TSV/R2 paths; these were
the deliberate changes made along the way:

- **SQLite storage backend has no Go equivalent.** See
  [Local SQLite Storage](#local-sqlite-storage-poc-for-cloudflare-d1) above —
  it's a Rust-only POC for a future Cloudflare D1 backend.

- **CLI flag syntax.** Go's `flag` package accepts `-once`; clap uses GNU-style
  `--once`, `--interval 10m`, `--data-dir ./data`. Local mode is `--r2 false`
  (Go: `-r2=false`).
- **Storage dispatch.** Go declares a `DataStore` interface and type-asserts at
  request time to discover whether the backend supports history. Rust uses a
  `Store` enum, so the capability check is an exhaustive `match` and
  "unsupported by this backend" is an explicit error variant that the web layer
  maps to HTTP 501 — the same status the Go server returns.
- **TSV code is shared.** The Go version duplicates the render and parse loops
  between `storage/tsv.go` and `storage/r2.go`; here both backends call
  `storage::tsvfmt`.
- **`/api/history` fetches snapshots concurrently** (16 at a time) instead of
  serially, then sorts newest-first to preserve the API's ordering contract.
- **`list_available_timestamps` on R2 reads timestamps from object keys**
  instead of downloading every snapshot to read its first row.
- **Lenient XML numbers.** The feed occasionally emits empty elements such as
  `<nbEBikes/>`; those deserialize to `0` rather than failing the whole fetch.
- **Live-fetch fallback timestamp.** When `/api/stations` has no stored data and
  falls back to the live TFL feed, the response is stamped with the current time.
  Go leaves the zero value there, which the UI renders as the year 0001.
- **`.env` is loaded before `USE_R2` is read**, so that variable can live in
  `.env` too; Go only reads it from the real environment.
- **Graceful shutdown** covers SIGINT and SIGTERM on Unix (Ctrl-C on Windows)
  for all three binaries, including in-flight HTTP requests on the server.
