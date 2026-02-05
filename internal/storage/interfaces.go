package storage

import (
	"time"

	"city-cycling/internal/tfl"
)

// DataStore is the interface for reading station data.
// It's implemented by both TSVStorage and R2Storage.
type DataStore interface {
	// ReadLatestStations reads the most recent station data.
	ReadLatestStations() ([]tfl.Station, time.Time, error)

	// ListAvailableTimestamps returns all available data timestamps.
	ListAvailableTimestamps() ([]time.Time, error)
}

// TSVDataStore is an interface for TSV-specific operations.
type TSVDataStore interface {
	DataStore
	// WriteStations writes station data to a file.
	WriteStations(stations *tfl.Stations) (string, error)
}

// R2DataStore is an interface for R2-specific operations.
type R2DataStore interface {
	DataStore
	// WriteStations writes station data to R2.
	WriteStations(ctx interface{}, stations *tfl.Stations) (string, error)

	// ListSnapshots returns all snapshot keys in R2.
	ListSnapshots(ctx interface{}) ([]string, error)

	// GetSnapshot downloads and parses a specific snapshot from R2.
	GetSnapshot(ctx interface{}, key string) ([]tfl.Station, time.Time, error)
}
