package main

import (
	"context"
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	"city-cycling/internal/config"
	"city-cycling/internal/storage"
	"city-cycling/internal/tfl"
)

func main() {
	var (
		interval = flag.Duration("interval", 5*time.Minute, "Fetch interval (set to 0 for one-shot mode)")
		oneShot  = flag.Bool("once", false, "Run once and exit")
	)
	flag.Parse()

	// Load R2 configuration from .env or environment variables
	cfg, err := config.LoadR2Config()
	if err != nil {
		log.Fatalf("Configuration error: %v", err)
	}

	// Log configuration (without secrets)
	log.Printf("R2 Configuration:")
	log.Printf("  Endpoint: %s", cfg.Endpoint)
	log.Printf("  Bucket: %s", cfg.BucketName)
	log.Printf("  Region: %s", cfg.Region)
	log.Printf("  Prefix: %s", cfg.Prefix)

	client := tfl.NewClient()
	store, err := storage.NewR2Storage(cfg.AccessKeyID, cfg.SecretAccessKey, cfg.Endpoint, cfg.BucketName, cfg.Region, cfg.Prefix)
	if err != nil {
		log.Fatalf("Failed to initialize R2 storage: %v", err)
	}

	ctx := context.Background()

	// Verify bucket exists - this helps catch configuration issues early
	log.Println("Verifying R2 bucket access...")
	exists, err := store.BucketExists(ctx)
	if err != nil {
		log.Fatalf("Bucket verification failed: %v", err)
	}
	if !exists {
		log.Fatalf("Bucket '%s' does not exist or is not accessible", cfg.BucketName)
	}
	log.Println("Bucket verified successfully")

	// Perform initial fetch
	if err := fetchAndStore(ctx, client, store); err != nil {
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
			if err := fetchAndStore(ctx, client, store); err != nil {
				log.Printf("Fetch failed: %v", err)
			}
		case sig := <-sigChan:
			log.Printf("Received signal %v, shutting down", sig)
			return
		}
	}
}

func fetchAndStore(ctx context.Context, client *tfl.Client, store *storage.R2Storage) error {
	log.Println("Fetching station data...")

	stations, err := client.FetchStations()
	if err != nil {
		return err
	}

	key, err := store.WriteStations(ctx, stations)
	if err != nil {
		return err
	}

	log.Printf("Uploaded %d stations to R2: %s", len(stations.Stations), key)
	return nil
}
