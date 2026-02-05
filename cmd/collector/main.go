package main

import (
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	"city-cycling/internal/storage"
	"city-cycling/internal/tfl"
)

func main() {
	var (
		dataDir  = flag.String("data-dir", "data", "Directory to store TSV files")
		interval = flag.Duration("interval", 5*time.Minute, "Fetch interval (set to 0 for one-shot mode)")
		oneShot  = flag.Bool("once", false, "Run once and exit")
	)
	flag.Parse()

	client := tfl.NewClient()
	store := storage.NewTSVStorage(*dataDir)

	// Perform initial fetch
	if err := fetchAndStore(client, store); err != nil {
		log.Fatalf("Initial fetch failed: %v", err)
	}

	// If one-shot mode, exit after first fetch
	if *oneShot || *interval == 0 {
		log.Println("One-shot mode: exiting after single fetch")
		return
	}

	// Set up signal handling for graceful shutdown
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	ticker := time.NewTicker(*interval)
	defer ticker.Stop()

	log.Printf("Collector running with %v interval. Press Ctrl+C to stop.", *interval)

	for {
		select {
		case <-ticker.C:
			if err := fetchAndStore(client, store); err != nil {
				log.Printf("Fetch failed: %v", err)
			}
		case sig := <-sigChan:
			log.Printf("Received signal %v, shutting down", sig)
			return
		}
	}
}

func fetchAndStore(client *tfl.Client, store *storage.TSVStorage) error {
	log.Println("Fetching station data...")

	stations, err := client.FetchStations()
	if err != nil {
		return err
	}

	filepath, err := store.WriteStations(stations)
	if err != nil {
		return err
	}

	log.Printf("Saved %d stations to %s", len(stations.Stations), filepath)
	return nil
}
