# City Cycling — Cloudflare Worker

A Cloudflare-native build of City Cycling: one Worker script with two
triggers — a `fetch` handler serving the map/API, and a `scheduled` handler
(Cron Trigger) running the collector — both reading and writing a
[D1](https://developers.cloudflare.com/d1/) database. D1 is SQLite, so this
reuses the exact schema designed for the [local SQLite POC](../README.md#local-sqlite-storage-poc-for-cloudflare-d1)
in the parent crate, swapping `rusqlite` for the Workers D1 binding.

This is a separate Cargo package from the parent crate's `axum`/`tokio`
binaries — the `worker` crate targets `wasm32-unknown-unknown` and has its
own request/response/async types (no `reqwest`, no `tokio`, no TCP
listener), so it can't share that crate's dependency graph.

**Data source differs from the tokio implementation.** The parent crate
fetches TfL's website XML syndication feed (`tfl.gov.uk/.../livecyclehireupdates.xml`).
This Worker fetches TfL's official JSON API instead
(`api.tfl.gov.uk/BikePoint/`) — see "TFL Fetch: Cloudflare Managed
Challenge" below for why. The D1 schema is identical either way; only
`src/tfl.rs`'s parsing differs (JSON `additionalProperties` key/value bag
instead of fixed XML elements), mapping onto the same `Station`/`Stations`
shape so nothing downstream (`db.rs`, `lib.rs`) needed to change.

## Architecture

```
cf-worker/
├── Cargo.toml
├── wrangler.toml          # Worker config: D1 binding, cron trigger, build command
├── migrations/
│   └── 0001_init.sql      # Same schema as the SQLite POC
└── src/
    ├── lib.rs             # #[event(fetch)] router + #[event(scheduled)] collector
    ├── db.rs               # D1 queries
    ├── tfl.rs              # TFL BikePoint JSON model + fetch (via worker::Fetch, not reqwest)
    └── map.html            # Map UI (embedded with include_str!)
```

- **`fetch`**: a `Router` with the same four routes as the tokio server —
  `/`, `/api/stations`, `/api/history`, `/api/history/snapshot` — same JSON
  field names, same "echo the requested timestamp back" behaviour for
  snapshot lookups. The existing frontend (`src/map.html`) works against it
  unmodified.
- **`scheduled`**: fires every 15 minutes (`wrangler.toml`'s
  `[triggers]`), fetches the TFL feed, and writes a snapshot to D1 — except
  during an **11pm–5am London-local pause**, to avoid piling up
  near-identical overnight snapshots. The cron itself fires continuously;
  the pause is a check inside the handler (`chrono-tz`, DST-aware), not a
  restricted UTC cron window — a fixed UTC window would drift by an hour
  across the BST/GMT transition and not actually land on 11pm–5am London
  time for half the year.

## Prerequisites

```bash
rustup target add wasm32-unknown-unknown
cargo install worker-build
```

`wrangler` itself doesn't need a separate install — `wrangler.toml`'s
`[build]` command installs `worker-build` and `npx wrangler` fetches
`wrangler` on first use. You do need [Node.js](https://nodejs.org/).

## Local Development

No Cloudflare account needed for this part — `--local` runs entirely against
an on-disk SQLite file under `.wrangler/state/`, not the real D1 service.

```bash
cd cf-worker

# Apply the schema to a fresh local D1 file
npx wrangler d1 migrations apply city-cycling-db --local

# Start the Worker locally (runs worker-build automatically, then serves on :8787)
npx wrangler dev --local --port 8787
```

Then, in another terminal:

```bash
curl http://127.0.0.1:8787/
curl http://127.0.0.1:8787/api/stations
curl http://127.0.0.1:8787/api/history
```

Cron Triggers don't fire on a timer in local dev. Trigger the collector by
hand instead:

```bash
curl http://127.0.0.1:8787/cdn-cgi/local/scheduled
```

Each call fetches the live TFL feed and writes one snapshot, so running it
a couple of times a second or more apart (`snapshots.captured_at` has a
`UNIQUE` constraint at one-second resolution) builds up enough history to
exercise `/api/history` and `/api/history/snapshot?timestamp=<rfc3339>`.

## Deploying to Cloudflare

These steps touch your real Cloudflare account — do them yourself from your
own terminal.

1. **Authenticate wrangler** (opens a browser):
   ```bash
   npx wrangler login
   ```

2. **Create the D1 database:**
   ```bash
   npx wrangler d1 create city-cycling-db
   ```
   This prints a `database_id`. Paste it into `wrangler.toml`'s
   `[[d1_databases]]` block, replacing `REPLACE_WITH_DATABASE_ID`.

3. **Apply the schema to the real database:**
   ```bash
   npx wrangler d1 migrations apply city-cycling-db --remote
   ```

4. **Deploy:**
   ```bash
   npx wrangler deploy
   ```
   This builds the Worker (via the `[build]` command in `wrangler.toml`)
   and uploads it along with the D1 binding and the Cron Trigger
   (`*/15 * * * *`, defined in `wrangler.toml`). Cloudflare starts firing the
   cron immediately on deploy — no separate step needed to "start" the
   collector.

5. **Verify:**
   ```bash
   npx wrangler tail          # watch live logs, including the collector's
   curl https://<your-worker>.<subdomain>.workers.dev/api/stations
   ```
   The first `/api/stations` call before the cron has fired yet falls back
   to a live TFL fetch (same as the tokio server's behaviour); subsequent
   calls read from D1 once the first cron run lands.

## API Endpoints

Identical contract to the tokio/axum server (see
[`../README.md`](../README.md#api-endpoints)):

- `GET /` — the map
- `GET /api/stations` — current station data
- `GET /api/history` — aggregate trends, newest first
- `GET /api/history/snapshot?timestamp=<rfc3339>` — snapshot closest to a timestamp

## TFL Fetch: Cloudflare Managed Challenge

The website's XML feed (what the tokio implementation still uses) didn't
work from this Worker in production, and it's worth recording why, since
the failure mode was misleading. In order:

1. **No request headers at all**: the collector's `quick_xml` parse failed
   with `expected </meta>, but </head> was found` — HTML markup, not
   anything in the TFL feed. TFL's endpoint had served back a landing/
   challenge page instead of the XML.
2. **A custom `User-Agent: city-cycling/1.0`**: got an explicit `403`. A
   descriptive, app-style User-Agent is exactly what simple bot-detection
   rules pattern-match on, so this made things *worse*.
3. **A realistic browser header set** (Chrome User-Agent, `Accept`,
   `Accept-Language`): still `403`, and — the key clue, confirmed by
   capturing the actual response via `wrangler tail`
   (`cf-mitigated: challenge`, `"cType":"managed"`, body containing
   `"Verification required... Enable JavaScript and cookies to continue"`)
   — this is **Cloudflare's Managed Challenge**, not a WAF header rule.
   TFL's `tfl.gov.uk` is itself fronted by Cloudflare, so a Worker calling
   it is a Cloudflare-edge-to-Cloudflare-zone subrequest, and that zone's
   bot management can flag it. A Managed Challenge requires executing
   JavaScript and passing a browser attestation — something no server-side
   HTTP client can do, so no header combination was ever going to fix it.
   It also explained an earlier puzzle: `/api/stations`' live-TFL-fallback
   path (identical code, identical headers) kept succeeding in production
   while the `scheduled` trigger kept hitting the challenge — Cloudflare
   scores each outbound connection somewhat independently (TLS handshake,
   egress IP from the shared Workers pool), so identical code can get
   different outcomes per invocation.

**Fix**: switched to TfL's official Unified API (`api.tfl.gov.uk/BikePoint/`)
— a JSON endpoint meant for machine consumption, as opposed to the website's
syndication feed. It's also Cloudflare-fronted but evidently doesn't carry
the same policy; verified working both locally and against the real
production Cron Trigger after deploying.

## What Was Actually Verified

Everything above was run for real, locally:

- `cargo check` / `worker-build --release` against `wasm32-unknown-unknown` — clean, no warnings.
- `wrangler d1 migrations apply --local` — schema applied to a local D1 file.
- `wrangler dev --local` — the Worker served real traffic: `/` (200, HTML),
  `/api/stations` (live TFL fallback with no stored data, then a correct D1
  read afterward), `/api/history` (correct aggregates, newest-first across
  two runs), `/api/history/snapshot` (exact match plus the two 400 error
  paths), and the collector ran for real against the live TFL feed via the
  manual `/cdn-cgi/local/scheduled` trigger, including a second run that
  exercised the station-metadata upsert path.
- After switching to `api.tfl.gov.uk/BikePoint/`: re-ran the same local
  checks against the live JSON API — correct parsing (station id extracted
  from `"BikePoints_<n>"`, bike/dock counts read out of the
  `additionalProperties` bag), a full collector write (797 real stations),
  and both read endpoints serving that data back correctly. `src/tfl.rs`
  also has unit tests for the JSON-to-`Station` mapping (missing-property
  default, full-response shape); these compile under `wasm32-unknown-unknown`
  but don't run there without a `wasm-bindgen-test` harness, which wasn't
  set up for this — the live local run is the stronger signal here anyway.

**Not verified**, because it requires your own Cloudflare account: an actual
`wrangler deploy`, the Cron Trigger firing on Cloudflare's real schedule, and
`wrangler d1 migrations apply --remote` against a production D1 database.
The local D1 emulation (`--local`) is the same SQLite engine D1 runs on
production, so this is a reasonably strong signal, but it isn't a substitute
for running the deploy steps above yourself.

## Notes

- **D1's typed integer binding (`D1Type::Integer`) is `i32`, not `i64`.**
  Station ids and bike counts are always small, so this is a non-issue there.
  The one place it would matter — binding a Unix timestamp for the
  nearest-snapshot lookup in `db.rs` — binds it as `D1Type::Real` (`f64`)
  instead, specifically to avoid the `i32` overflow that would otherwise hit
  in 2038.
- **Snapshot writes use one `batch()` call** for all ~800 stations' metadata
  upserts and readings (plus the aggregate row) rather than one `INSERT`
  per table. This matters because D1 caps bound parameters at 100 *per
  statement* — a single multi-row `INSERT ... VALUES (...), (...), ...` for
  800 stations would blow past that — but a `batch()` of ~1,600 small
  statements counts as one query against the per-invocation query limit, so
  it's both correct and cheap.
- This backend has no Go equivalent and is independent of the SQLite POC in
  `src/storage/sqlite.rs` at the repo root — that one is for the existing
  tokio/axum server; this one is the actual Cloudflare deployment target.
