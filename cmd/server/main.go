package main

import (
	"flag"
	"fmt"
	"log"
	"net/http"

	"city-cycling/internal/storage"
	"city-cycling/internal/tfl"
	"city-cycling/internal/web"
)

func main() {
	var (
		port    = flag.Int("port", 8080, "HTTP server port")
		dataDir = flag.String("data-dir", "data", "Directory containing TSV data files")
	)
	flag.Parse()

	store := storage.NewTSVStorage(*dataDir)
	tflClient := tfl.NewClient()

	handler, err := web.NewHandler(store, tflClient)
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
