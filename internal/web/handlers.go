package web

import (
	"embed"
	"encoding/json"
	"html/template"
	"log"
	"net/http"
	"sync"
	"time"

	"city-cycling/internal/storage"
	"city-cycling/internal/tfl"
)

const (
	// historyCacheTTL is how long to cache the aggregate historical data.
	historyCacheTTL = 10 * time.Minute
)

//go:embed templates/*
var templatesFS embed.FS

// StationResponse is the JSON response format for a single station.
type StationResponse struct {
	ID              int     `json:"id"`
	Name            string  `json:"name"`
	Lat             float64 `json:"lat"`
	Long            float64 `json:"lng"`
	NbBikes         int     `json:"nbBikes"`
	NbStandardBikes int     `json:"nbStandardBikes"`
	NbEBikes        int     `json:"nbEBikes"`
	NbEmptyDocks    int     `json:"nbEmptyDocks"`
	NbDocks         int     `json:"nbDocks"`
}

// StationsResponse is the JSON response for the stations API.
type StationsResponse struct {
	Timestamp string            `json:"timestamp"`
	Stations  []StationResponse `json:"stations"`
}

// HistoryDataPointResponse represents historical aggregate data at a point in time.
type HistoryDataPointResponse struct {
	Timestamp       string `json:"timestamp"`
	TotalBikes      int    `json:"totalBikes"`
	TotalEBikes     int    `json:"totalEBikes"`
	TotalEmptyDocks int    `json:"totalEmptyDocks"`
	StationCount    int    `json:"stationCount"`
}

// HistoryResponse is the JSON response for the history API.
type HistoryResponse struct {
	DataPoints []HistoryDataPointResponse `json:"dataPoints"`
}

// Handler provides HTTP handlers for the web interface.
type Handler struct {
	store     storage.DataStore
	tflClient *tfl.Client
	templates *template.Template

	// Cache for historical data
	historyCache     []storage.HistoricalDataPoint
	historyCacheTime time.Time
	historyCacheMu   sync.RWMutex

	// Cache for snapshots by timestamp (immutable, no TTL needed)
	snapshotCache   map[string][]tfl.Station
	snapshotCacheMu sync.RWMutex
}

// NewHandler creates a new web handler.
func NewHandler(store storage.DataStore, tflClient *tfl.Client) (*Handler, error) {
	tmpl, err := template.ParseFS(templatesFS, "templates/*.html")
	if err != nil {
		return nil, err
	}

	return &Handler{
		store:         store,
		tflClient:     tflClient,
		templates:     tmpl,
		snapshotCache: make(map[string][]tfl.Station),
	}, nil
}

// RegisterRoutes registers all HTTP routes on the given mux.
func (h *Handler) RegisterRoutes(mux *http.ServeMux) {
	mux.HandleFunc("/", h.handleMap)
	mux.HandleFunc("/api/stations", h.handleStations)
	mux.HandleFunc("/api/history", h.handleHistory)
	mux.HandleFunc("/api/history/snapshot", h.handleHistorySnapshot)
}

// handleMap serves the main map page.
func (h *Handler) handleMap(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}

	if err := h.templates.ExecuteTemplate(w, "map.html", nil); err != nil {
		log.Printf("Template error: %v", err)
		http.Error(w, "Internal server error", http.StatusInternalServerError)
	}
}

// handleStations serves the stations API endpoint.
func (h *Handler) handleStations(w http.ResponseWriter, r *http.Request) {
	// Try to read from storage first
	stations, timestamp, err := h.store.ReadLatestStations()
	if err != nil {
		// Fall back to live API if no stored data
		log.Printf("No stored data, fetching live: %v", err)
		liveData, err := h.tflClient.FetchStations()
		if err != nil {
			http.Error(w, "Failed to fetch station data", http.StatusInternalServerError)
			return
		}
		stations = liveData.Stations
	}

	response := StationsResponse{
		Timestamp: timestamp.Format("2006-01-02T15:04:05Z"),
		Stations:  make([]StationResponse, len(stations)),
	}

	for i, s := range stations {
		response.Stations[i] = StationResponse{
			ID:              s.ID,
			Name:            s.Name,
			Lat:             s.Lat,
			Long:            s.Long,
			NbBikes:         s.NbBikes,
			NbStandardBikes: s.NbStandardBikes,
			NbEBikes:        s.NbEBikes,
			NbEmptyDocks:    s.NbEmptyDocks,
			NbDocks:         s.NbDocks,
		}
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(response); err != nil {
		log.Printf("JSON encoding error: %v", err)
	}
}

// handleHistory serves historical usage data.
func (h *Handler) handleHistory(w http.ResponseWriter, r *http.Request) {
	// Check if store supports historical data
	historicalStore, ok := h.store.(storage.HistoricalDataStore)
	if !ok {
		http.Error(w, "Historical data not available with current storage backend", http.StatusNotImplemented)
		return
	}

	// Check cache first
	h.historyCacheMu.RLock()
	if h.historyCache != nil && time.Since(h.historyCacheTime) < historyCacheTTL {
		dataPoints := h.historyCache
		h.historyCacheMu.RUnlock()
		log.Printf("History cache hit (%d data points)", len(dataPoints))
		h.writeHistoryResponse(w, dataPoints)
		return
	}
	h.historyCacheMu.RUnlock()

	// Cache miss - fetch from storage
	ctx := r.Context()
	dataPoints, err := historicalStore.GetHistoricalData(ctx)
	if err != nil {
		log.Printf("Failed to get historical data: %v", err)
		http.Error(w, "Failed to fetch historical data", http.StatusInternalServerError)
		return
	}

	// Update cache
	h.historyCacheMu.Lock()
	h.historyCache = dataPoints
	h.historyCacheTime = time.Now()
	h.historyCacheMu.Unlock()
	log.Printf("History cache updated (%d data points)", len(dataPoints))

	h.writeHistoryResponse(w, dataPoints)
}

// writeHistoryResponse writes the history response JSON.
func (h *Handler) writeHistoryResponse(w http.ResponseWriter, dataPoints []storage.HistoricalDataPoint) {
	response := HistoryResponse{
		DataPoints: make([]HistoryDataPointResponse, len(dataPoints)),
	}

	for i, dp := range dataPoints {
		response.DataPoints[i] = HistoryDataPointResponse{
			Timestamp:       dp.Timestamp.Format("2006-01-02T15:04:05Z"),
			TotalBikes:      dp.TotalBikes,
			TotalEBikes:     dp.TotalEBikes,
			TotalEmptyDocks: dp.TotalEmptyDocks,
			StationCount:    dp.StationCount,
		}
	}

	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "public, max-age=3600") // Cache for 1 hour
	if err := json.NewEncoder(w).Encode(response); err != nil {
		log.Printf("JSON encoding error: %v", err)
	}
}

// handleHistorySnapshot serves station data for a specific timestamp from historical snapshots.
func (h *Handler) handleHistorySnapshot(w http.ResponseWriter, r *http.Request) {
	// Get timestamp from query parameter
	timestampStr := r.URL.Query().Get("timestamp")
	if timestampStr == "" {
		http.Error(w, "Missing timestamp parameter", http.StatusBadRequest)
		return
	}

	// Parse timestamp
	targetTime, err := time.Parse(time.RFC3339, timestampStr)
	if err != nil {
		http.Error(w, "Invalid timestamp format", http.StatusBadRequest)
		return
	}

	// Use normalized timestamp string as cache key
	cacheKey := targetTime.UTC().Format(time.RFC3339)

	// Check cache first (snapshots are immutable, no TTL needed)
	h.snapshotCacheMu.RLock()
	if stations, ok := h.snapshotCache[cacheKey]; ok {
		h.snapshotCacheMu.RUnlock()
		log.Printf("Snapshot cache hit for %s (%d stations)", cacheKey, len(stations))
		h.writeSnapshotResponse(w, targetTime, stations)
		return
	}
	h.snapshotCacheMu.RUnlock()

	// Check if store supports R2 operations
	r2Store, ok := h.store.(storage.R2DataStore)
	if !ok {
		http.Error(w, "Historical snapshot data not available with current storage backend", http.StatusNotImplemented)
		return
	}

	// Cache miss - fetch from storage
	ctx := r.Context()
	stations, err := r2Store.GetSnapshotByTimestamp(ctx, targetTime)
	if err != nil {
		log.Printf("Failed to get snapshot for timestamp %s: %v", timestampStr, err)
		http.Error(w, "Failed to fetch snapshot data", http.StatusInternalServerError)
		return
	}

	// Update cache
	h.snapshotCacheMu.Lock()
	h.snapshotCache[cacheKey] = stations
	h.snapshotCacheMu.Unlock()
	log.Printf("Snapshot cache updated for %s (%d stations)", cacheKey, len(stations))

	h.writeSnapshotResponse(w, targetTime, stations)
}

// writeSnapshotResponse writes the snapshot response JSON.
func (h *Handler) writeSnapshotResponse(w http.ResponseWriter, timestamp time.Time, stations []tfl.Station) {
	response := StationsResponse{
		Timestamp: timestamp.Format("2006-01-02T15:04:05Z"),
		Stations:  make([]StationResponse, len(stations)),
	}

	for i, s := range stations {
		response.Stations[i] = StationResponse{
			ID:              s.ID,
			Name:            s.Name,
			Lat:             s.Lat,
			Long:            s.Long,
			NbBikes:         s.NbBikes,
			NbStandardBikes: s.NbStandardBikes,
			NbEBikes:        s.NbEBikes,
			NbEmptyDocks:    s.NbEmptyDocks,
			NbDocks:         s.NbDocks,
		}
	}

	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "public, max-age=604800, immutable") // Cache for 1 week (snapshots are immutable)
	if err := json.NewEncoder(w).Encode(response); err != nil {
		log.Printf("JSON encoding error: %v", err)
	}
}
