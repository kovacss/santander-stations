package config

import (
	"fmt"
	"os"

	"github.com/joho/godotenv"
)

// R2Config holds Cloudflare R2 configuration.
type R2Config struct {
	AccessKeyID     string
	SecretAccessKey string
	Endpoint        string
	BucketName      string
	Prefix          string
	Region          string
}

// LoadR2Config loads R2 configuration from environment variables or .env file.
// For local development, it attempts to load from .env file first.
// For production, it relies on environment variables set by the platform.
func LoadR2Config() (*R2Config, error) {
	// Try to load .env file (only for local development)
	// Ignore error if file doesn't exist (expected in production)
	_ = godotenv.Load()

	accessKeyID := os.Getenv("S3_ACCESS_KEY_ID")
	secretAccessKey := os.Getenv("S3_SECRET_ACCESS_KEY")
	endpoint := os.Getenv("S3_ENDPOINT")
	bucketName := os.Getenv("S3_BUCKET_NAME")
	prefix := os.Getenv("S3_PREFIX")
	region := os.Getenv("S3_REGION")

	if prefix == "" {
		prefix = "snapshots/"
	}

	// For Cloudflare R2, region doesn't matter but AWS SDK requires it
	// Default to "auto" if not specified
	if region == "" {
		region = "auto"
	}

	// Validate required fields
	var missing []string
	if accessKeyID == "" {
		missing = append(missing, "S3_ACCESS_KEY_ID")
	}
	if secretAccessKey == "" {
		missing = append(missing, "S3_SECRET_ACCESS_KEY")
	}
	if endpoint == "" {
		missing = append(missing, "S3_ENDPOINT")
	}
	if bucketName == "" {
		missing = append(missing, "S3_BUCKET_NAME")
	}

	if len(missing) > 0 {
		return nil, fmt.Errorf("missing required environment variables: %v", missing)
	}

	return &R2Config{
		AccessKeyID:     accessKeyID,
		SecretAccessKey: secretAccessKey,
		Endpoint:        endpoint,
		BucketName:      bucketName,
		Prefix:          prefix,
		Region:          region,
	}, nil
}
