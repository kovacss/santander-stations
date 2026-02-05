package main

import (
	"flag"
	"fmt"
	"log"
	"net/http"
	"os"

	"city-cycling/internal/config"
	"city-cycling/internal/storage"
	"city-cycling/internal/tfl"
	"city-cycling/internal/web"
)

func main() {
	var (
		port    = flag.Int("port", 8080, "HTTP server port")
		dataDir = flag.String("data-dir", "data", "Directory containing TSV data files (local mode only)")
		useR2   = flag.Bool("r2", true, "Use Cloudflare R2 for data storage (default: local files)")
	)
	flag.Parse()

	// Allow overriding via environment variable
	if os.Getenv("USE_R2") != "" {
		*useR2 = true
	}
	// Allow overriding port via environment variable
	if portEnv := os.Getenv("PORT"); portEnv != "" {
		fmt.Sscanf(portEnv, "%d", port)
	}

	var dataStore storage.DataStore
	var err error

	if *useR2 {
		// Initialize R2 storage for production
		log.Println("Using Cloudflare R2 for data storage")
		cfg, err := config.LoadR2Config()
		if err != nil {
			log.Fatalf("Failed to load R2 config: %v", err)
		}

		dataStore, err = storage.NewR2Storage(
			cfg.AccessKeyID,
			cfg.SecretAccessKey,
			cfg.Endpoint,
			cfg.BucketName,
			cfg.Region,
			cfg.Prefix,
		)
		if err != nil {
			log.Fatalf("Failed to initialize R2 storage: %v", err)
		}

		log.Printf("R2 Bucket: %s", cfg.BucketName)
	} else {
		// Initialize local file storage for development
		log.Println("Using local file storage")
		dataStore = storage.NewTSVStorage(*dataDir)
		log.Printf("Data directory: %s", *dataDir)
	}

	tflClient := tfl.NewClient()

	handler, err := web.NewHandler(dataStore, tflClient)
	if err != nil {
		log.Fatalf("Failed to create handler: %v", err)
	}

	mux := http.NewServeMux()
	handler.RegisterRoutes(mux)

	addr := fmt.Sprintf(":%d", *port)
	log.Printf("Starting server on http://localhost%s", addr)

	if err := http.ListenAndServe(addr, mux); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}
