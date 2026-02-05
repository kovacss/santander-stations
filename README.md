# City Cycling

A Go application that tracks Santander Cycle stations in London in real-time and provides an interactive web-based map interface.

## Features

- **Real-time Data Collection**: Fetches current station data from the Transport for London (TFL) API
- **Historical Tracking**: Stores timestamped snapshots of station data in TSV format
- **Interactive Map**: Web-based UI displaying all stations on a map with real-time availability
- **Color-Coded Status**:
  - Green: Bikes available
  - Yellow: Low availability
  - Red: No bikes available
- **Station Details**: Click any marker to view bike counts and dock information
- **Trends & Analytics**: View historical usage patterns with interactive charts showing bike availability over time

## Project Structure

```
city-cycling/
├── cmd/
│   ├── collector/main.go   # Data collection CLI
│   └── server/main.go      # Web server
├── internal/
│   ├── tfl/
│   │   ├── client.go       # TFL API HTTP client
│   │   └── models.go       # XML parsing structures
│   ├── storage/tsv.go      # TSV file operations
│   └── web/
│       ├── handlers.go     # HTTP request handlers
│       └── templates/map.html
├── data/                   # TSV data storage (auto-created)
└── go.mod
```

## Installation

Requires Go 1.21 or higher.

```bash
# Clone the repository
git clone <repository-url>
cd city-cycling

# Install dependencies
go mod download
```

## Usage

### Local Data Collector

Fetch and store station data to local filesystem:

```bash
# One-shot fetch
go run ./cmd/collector -once

# Continuous mode (default 5-minute interval)
go run ./cmd/collector

# Custom interval (e.g., every 10 minutes)
go run ./cmd/collector -interval 10m
```

The collector creates timestamped TSV files in the `data/` directory.

### Cloudflare R2 Data Collector

For production deployments, use the R2 collector to upload data to Cloudflare R2:

**Local Development (using .env file):**

1. Copy `.env.example` to `.env`:
   ```bash
   cp .env.example .env
   ```

2. Edit `.env` with your Cloudflare R2 credentials:
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
   go run ./cmd/collector-r2 -once

   # Continuous mode with custom interval
   go run ./cmd/collector-r2 -interval 5m
   ```

**Production (using environment variables):**

Railway will handle setting environment variables, so just run:
```bash
go run ./cmd/collector-r2 -once
```

The collector stores data using the same TSV format with columns:
- `timestamp`: ISO 8601 timestamp of the fetch
- `id`: Station ID
- `name`: Station name
- `lat`: Latitude coordinate
- `long`: Longitude coordinate
- `nb_bikes`: Total bikes available
- `nb_standard_bikes`: Standard bikes
- `nb_ebikes`: E-bikes
- `nb_empty_docks`: Empty docks
- `nb_docks`: Total docks

### Web Server

Start the interactive map server:

**Local Development (reads from local TSV files):**
```bash
go run ./cmd/server
```

**Production (reads from Cloudflare R2):**
```bash
USE_R2=true go run ./cmd/server
```

Or set environment variables:
```bash
export S3_ACCESS_KEY_ID=your_key
export S3_SECRET_ACCESS_KEY=your_secret
export S3_ENDPOINT=https://xxxxx.r2.cloudflarestorage.com
export S3_BUCKET_NAME=your-bucket
export S3_REGION=auto
export USE_R2=true

go run ./cmd/server
```

The server will start at `http://localhost:8080` and display an interactive map showing all 800 Santander Cycle stations with the latest data from your configured storage backend.

## API Endpoints

- `GET /` - Serves the interactive map interface
- `GET /api/stations` - Returns current station data as JSON
- `GET /api/history` - Returns historical usage trends over time aggregated from all snapshots (R2 backend only)

### History API Response Format

The `/api/history` endpoint returns aggregate statistics from all available snapshots:

```json
{
  "dataPoints": [
    {
      "timestamp": "2026-02-05T16:45:00Z",
      "totalBikes": 5432,
      "totalEBikes": 892,
      "totalEmptyDocks": 4120,
      "stationCount": 800
    },
    ...
  ]
}
```

This can be used to visualize trends in bike availability over time.

## Data Format

Station data is stored in tab-separated values (TSV) format for easy analysis and historical tracking.

Example:
```
timestamp	id	name	lat	long	nb_bikes	nb_standard_bikes	nb_ebikes	nb_empty_docks	nb_docks
2026-02-05T14:47:14Z	1	River Street , Clerkenwell	51.529163	-0.109971	0	0	0	10	19
2026-02-05T14:47:14Z	2	Phillimore Gardens, Kensington	51.499607	-0.197574	3	1	2	29	37
```

## Technical Details

- **API**: Transport for London Unified API (BikePoint)
- **Data Format**: XML (parsed from TFL API)
- **Web Framework**: Standard Go `net/http`
- **Mapping**: Leaflet.js with OpenStreetMap tiles
- **Storage**: TSV files + Cloudflare R2 (production)

## Local Development Workflow

1. Start the data collector in the background:
   ```bash
   go run ./cmd/collector &
   ```

2. Start the web server in another terminal:
   ```bash
   go run ./cmd/server
   ```

3. Open `http://localhost:8080` in your browser

4. The map updates as new data is collected

## Deployment (Railway + Cloudflare R2)

### Architecture
- **Web Server**: Railway.app (runs Go HTTP server)
- **Data Collector**: Railway.app (runs via cron job, every 5 minutes)
- **Storage**: Cloudflare R2 (immutable historical TSV files)

### Deployment Checklist

- [x] **1. Set up Cloudflare R2 bucket**
  - [x] Create Cloudflare account (or use existing)
  - [x] Create R2 bucket (e.g., `city-cycling-data`)
  - [x] Generate API token with R2 access (`S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`)
  - [x] Note the bucket URL endpoint

- [x] **2. Configure R2 collector**
  - [x] Create R2 storage package with AWS SDK v2
  - [x] Create collector-r2 command (separate from local collector)
  - [x] Add .env file support (godotenv)
  - [x] Test locally with .env file

- [x] **3. Create Railway project**
  - [x] Sign up at [Railway.app](https://railway.app)
  - [x] Create new project
  - [x] Connect GitHub repository

- [x] **4. Update web server**
  - [x] Create DataStore interface (supports both local and R2)
  - [x] Add R2 support to R2Storage (ReadLatestStations, ListAvailableTimestamps)
  - [x] Update Handler to use DataStore interface
  - [x] Update server to auto-detect storage backend via USE_R2 env var
  - [x] Support both local file and R2 modes

- [x] **5. Add historical data API**
  - [x] Create HistoricalDataStore interface
  - [x] Implement GetHistoricalData in R2Storage (aggregate statistics across all snapshots)
  - [x] Add /api/history endpoint
  - [x] Return time-series data showing usage trends from start date to now

- [x] **6. Update frontend for trends visualization**
  - [x] Add Chart.js library for data visualization
  - [x] Create modal dialog for history view
  - [x] Add "View Trends" button to info panel
  - [x] Fetch historical data from /api/history
  - [x] Display line chart with Total Bikes, E-Bikes, Empty Docks
  - [x] Show summary statistics (current, average, peak, lowest)
  - [x] Error handling for missing historical data

- [ ] **7. Configure Railway environment variables**
  - [ ] Set `S3_ACCESS_KEY_ID` (from R2)
  - [ ] Set `S3_SECRET_ACCESS_KEY` (from R2)
  - [ ] Set `S3_BUCKET_NAME` (e.g., `city-cycling-data`)
  - [ ] Set `S3_ENDPOINT` (Cloudflare R2 endpoint)
  - [ ] Set `S3_REGION` (default: `auto`)
  - [ ] Set `USE_R2` to `true`
  - [ ] Set `PORT` to `8080`

- [ ] **7. Deploy web server**
  - [ ] Create service in Railway with Go buildpack
  - [ ] Set start command: `go run ./cmd/server`
  - [ ] Deploy

- [ ] **8. Set up cron job for collector**
  - [ ] Create separate Railway service for collector
  - [ ] Set start command: `go run ./cmd/collector-r2 -once`
  - [ ] Configure Railway Cron: every 5 minutes (`*/5 * * * *`)
  - [ ] Deploy

- [ ] **9. Verify deployment**
  - [ ] Check web server is live
  - [ ] Verify TSV files are being uploaded to R2
  - [ ] Check cron job execution in Railway logs
  - [ ] Test /api/history endpoint to verify historical data is accessible

### Cost Estimate
- **Railway**: ~$5-10/month (for both services + free tier cushion)
- **Cloudflare R2**: ~$0.30/month (for TSV storage at typical scale)
- **Total**: <$15/month

## Future Enhancements

- Trend analysis and heatmaps by time of day/week
- Station capacity predictions using historical trends
- Mobile app support
- Real-time updates via WebSocket
- Time-series analytics dashboards
- Export historical data as CSV/JSON
