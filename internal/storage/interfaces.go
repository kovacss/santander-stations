package storage

import (
	"context"
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

// HistoricalDataStore extends DataStore with methods for accessing historical data.
type HistoricalDataStore interface {
	DataStore

	// GetHistoricalData returns aggregate statistics for all available snapshots.
	// This is used to display trends over time.
	GetHistoricalData(ctx context.Context) ([]HistoricalDataPoint, error)
}

// TSVDataStore is an interface for TSV-specific operations.
type TSVDataStore interface {
	DataStore
	// WriteStations writes station data to a file.
	WriteStations(stations *tfl.Stations) (string, error)
}

// R2DataStore is an interface for R2-specific operations.
type R2DataStore interface {
	HistoricalDataStore
	// WriteStations writes station data to R2.
	WriteStations(ctx context.Context, stations *tfl.Stations) (string, error)

	// ListSnapshots returns all snapshot keys in R2.
	ListSnapshots(ctx context.Context) ([]string, error)

	// GetSnapshot downloads and parses a specific snapshot from R2.
	GetSnapshot(ctx context.Context, key string) ([]tfl.Station, time.Time, error)

	// GetSnapshotByTimestamp returns station data for a specific timestamp.
	GetSnapshotByTimestamp(ctx context.Context, timestamp time.Time) ([]tfl.Station, error)
}
