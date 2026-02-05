package web

import (
	"embed"
	"encoding/json"
	"html/template"
	"log"
	"net/http"

	"city-cycling/internal/storage"
	"city-cycling/internal/tfl"
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

// Handler provides HTTP handlers for the web interface.
type Handler struct {
	store     storage.DataStore
	tflClient *tfl.Client
	templates *template.Template
}

// NewHandler creates a new web handler.
func NewHandler(store storage.DataStore, tflClient *tfl.Client) (*Handler, error) {
	tmpl, err := template.ParseFS(templatesFS, "templates/*.html")
	if err != nil {
		return nil, err
	}

	return &Handler{
		store:     store,
		tflClient: tflClient,
		templates: tmpl,
	}, nil
}

// RegisterRoutes registers all HTTP routes on the given mux.
func (h *Handler) RegisterRoutes(mux *http.ServeMux) {
	mux.HandleFunc("/", h.handleMap)
	mux.HandleFunc("/api/stations", h.handleStations)
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
