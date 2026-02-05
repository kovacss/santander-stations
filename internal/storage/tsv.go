package storage

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"city-cycling/internal/tfl"
)

const (
	// TSVHeader defines the column headers for the TSV file.
	TSVHeader = "timestamp\tid\tname\tlat\tlong\tnb_bikes\tnb_standard_bikes\tnb_ebikes\tnb_empty_docks\tnb_docks"
)

// TSVStorage handles reading and writing station data to TSV files.
type TSVStorage struct {
	dataDir string
}

// NewTSVStorage creates a new TSV storage instance.
func NewTSVStorage(dataDir string) *TSVStorage {
	return &TSVStorage{dataDir: dataDir}
}

// WriteStations writes station data to a timestamped TSV file.
func (s *TSVStorage) WriteStations(stations *tfl.Stations) (string, error) {
	if err := os.MkdirAll(s.dataDir, 0755); err != nil {
		return "", fmt.Errorf("failed to create data directory: %w", err)
	}

	timestamp := time.Now().UTC()
	filename := fmt.Sprintf("stations_%s.tsv", timestamp.Format("20060102_150405"))
	filepath := filepath.Join(s.dataDir, filename)

	file, err := os.Create(filepath)
	if err != nil {
		return "", fmt.Errorf("failed to create file: %w", err)
	}
	defer file.Close()

	writer := bufio.NewWriter(file)

	// Write header
	if _, err := writer.WriteString(TSVHeader + "\n"); err != nil {
		return "", fmt.Errorf("failed to write header: %w", err)
	}

	// Write station data
	tsStr := timestamp.Format(time.RFC3339)
	for _, station := range stations.Stations {
		line := fmt.Sprintf("%s\t%d\t%s\t%.6f\t%.6f\t%d\t%d\t%d\t%d\t%d\n",
			tsStr,
			station.ID,
			strings.ReplaceAll(station.Name, "\t", " "), // Escape tabs in name
			station.Lat,
			station.Long,
			station.NbBikes,
			station.NbStandardBikes,
			station.NbEBikes,
			station.NbEmptyDocks,
			station.NbDocks,
		)
		if _, err := writer.WriteString(line); err != nil {
			return "", fmt.Errorf("failed to write station: %w", err)
		}
	}

	if err := writer.Flush(); err != nil {
		return "", fmt.Errorf("failed to flush writer: %w", err)
	}

	return filepath, nil
}

// ReadLatestStations reads the most recent TSV file and returns the stations.
func (s *TSVStorage) ReadLatestStations() ([]tfl.Station, time.Time, error) {
	files, err := s.listTSVFiles()
	if err != nil {
		return nil, time.Time{}, err
	}

	if len(files) == 0 {
		return nil, time.Time{}, fmt.Errorf("no station data files found")
	}

	// Files are sorted newest first
	return s.readTSVFile(files[0])
}

// ListAvailableTimestamps returns all timestamps for which data is available.
func (s *TSVStorage) ListAvailableTimestamps() ([]time.Time, error) {
	files, err := s.listTSVFiles()
	if err != nil {
		return nil, err
	}

	timestamps := make([]time.Time, 0, len(files))
	for _, file := range files {
		ts, err := s.parseFilenameTimestamp(file)
		if err == nil {
			timestamps = append(timestamps, ts)
		}
	}

	return timestamps, nil
}

// listTSVFiles returns TSV files sorted by timestamp (newest first).
func (s *TSVStorage) listTSVFiles() ([]string, error) {
	entries, err := os.ReadDir(s.dataDir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("failed to read data directory: %w", err)
	}

	var files []string
	for _, entry := range entries {
		if !entry.IsDir() && strings.HasPrefix(entry.Name(), "stations_") && strings.HasSuffix(entry.Name(), ".tsv") {
			files = append(files, filepath.Join(s.dataDir, entry.Name()))
		}
	}

	// Sort by filename (which contains timestamp) in descending order
	sort.Sort(sort.Reverse(sort.StringSlice(files)))

	return files, nil
}

// parseFilenameTimestamp extracts the timestamp from a TSV filename.
func (s *TSVStorage) parseFilenameTimestamp(filepath string) (time.Time, error) {
	base := strings.TrimSuffix(strings.TrimPrefix(filepath, s.dataDir+string(os.PathSeparator)), ".tsv")
	base = strings.TrimPrefix(base, "stations_")
	return time.Parse("20060102_150405", base)
}

// readTSVFile reads a TSV file and returns the stations.
func (s *TSVStorage) readTSVFile(filepath string) ([]tfl.Station, time.Time, error) {
	file, err := os.Open(filepath)
	if err != nil {
		return nil, time.Time{}, fmt.Errorf("failed to open file: %w", err)
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)

	// Skip header
	if !scanner.Scan() {
		return nil, time.Time{}, fmt.Errorf("empty file")
	}

	var stations []tfl.Station
	var timestamp time.Time
	var firstRow = true

	for scanner.Scan() {
		line := scanner.Text()
		fields := strings.Split(line, "\t")
		if len(fields) < 10 {
			continue
		}

		if firstRow {
			timestamp, _ = time.Parse(time.RFC3339, fields[0])
			firstRow = false
		}

		id, _ := strconv.Atoi(fields[1])
		lat, _ := strconv.ParseFloat(fields[3], 64)
		long, _ := strconv.ParseFloat(fields[4], 64)
		nbBikes, _ := strconv.Atoi(fields[5])
		nbStandardBikes, _ := strconv.Atoi(fields[6])
		nbEBikes, _ := strconv.Atoi(fields[7])
		nbEmptyDocks, _ := strconv.Atoi(fields[8])
		nbDocks, _ := strconv.Atoi(fields[9])

		stations = append(stations, tfl.Station{
			ID:              id,
			Name:            fields[2],
			Lat:             lat,
			Long:            long,
			NbBikes:         nbBikes,
			NbStandardBikes: nbStandardBikes,
			NbEBikes:        nbEBikes,
			NbEmptyDocks:    nbEmptyDocks,
			NbDocks:         nbDocks,
		})
	}

	if err := scanner.Err(); err != nil {
		return nil, time.Time{}, fmt.Errorf("error reading file: %w", err)
	}

	return stations, timestamp, nil
}
