package storage

import (
	"bufio"
	"bytes"
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"

	"city-cycling/internal/tfl"
)

// R2Storage handles reading and writing station data to Cloudflare R2.
type R2Storage struct {
	client *s3.Client
	bucket string
	prefix string
}

// NewR2Storage creates a new R2 storage instance.
// accessKeyID, secretAccessKey, endpoint, and region are required Cloudflare R2 credentials.
// prefix is optional and defaults to "snapshots/".
func NewR2Storage(accessKeyID, secretAccessKey, endpoint, bucket, region, prefix string) (*R2Storage, error) {
	if prefix == "" {
		prefix = "snapshots/"
	}

	// Create credentials provider
	credProvider := credentials.NewStaticCredentialsProvider(accessKeyID, secretAccessKey, "")

	// Create S3 client configured for Cloudflare R2
	client := s3.New(s3.Options{
		Credentials:  credProvider,
		BaseEndpoint: aws.String(endpoint),
		Region:       region,
		UsePathStyle: true,
	})

	return &R2Storage{
		client: client,
		bucket: bucket,
		prefix: prefix,
	}, nil
}

// WriteStations writes station data to R2 as a timestamped TSV file.
func (r *R2Storage) WriteStations(ctx context.Context, stations *tfl.Stations) (string, error) {
	timestamp := time.Now().UTC()
	key := fmt.Sprintf("%sstations_%s.tsv", r.prefix, timestamp.Format("20060102_150405"))

	// Build TSV content in memory
	var buf bytes.Buffer
	writer := bufio.NewWriter(&buf)

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

	// Upload to R2
	_, err := r.client.PutObject(ctx, &s3.PutObjectInput{
		Bucket:      aws.String(r.bucket),
		Key:         aws.String(key),
		Body:        bytes.NewReader(buf.Bytes()),
		ContentType: aws.String("text/tab-separated-values"),
		Metadata: map[string]string{
			"timestamp": tsStr,
			"stations":  fmt.Sprintf("%d", len(stations.Stations)),
		},
	})
	if err != nil {
		return "", fmt.Errorf("failed to upload to R2: %w", err)
	}

	return key, nil
}

// ListSnapshots returns all snapshot objects in R2, sorted by timestamp (newest first).
func (r *R2Storage) ListSnapshots(ctx context.Context) ([]string, error) {
	paginator := s3.NewListObjectsV2Paginator(r.client, &s3.ListObjectsV2Input{
		Bucket: aws.String(r.bucket),
		Prefix: aws.String(r.prefix),
	})

	var keys []string
	for paginator.HasMorePages() {
		result, err := paginator.NextPage(ctx)
		if err != nil {
			return nil, fmt.Errorf("failed to list objects: %w", err)
		}

		for _, obj := range result.Contents {
			keys = append(keys, aws.ToString(obj.Key))
		}
	}

	// Sort by key in descending order (newest first)
	// Since format is "snapshots/stations_YYYYMMDD_HHMMSS.tsv", reverse alphabetical sort works
	for i := len(keys)/2 - 1; i >= 0; i-- {
		j := len(keys) - 1 - i
		keys[i], keys[j] = keys[j], keys[i]
	}

	return keys, nil
}

// GetSnapshot downloads and parses a specific snapshot from R2.
func (r *R2Storage) GetSnapshot(ctx context.Context, key string) ([]tfl.Station, time.Time, error) {
	result, err := r.client.GetObject(ctx, &s3.GetObjectInput{
		Bucket: aws.String(r.bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		return nil, time.Time{}, fmt.Errorf("failed to get object: %w", err)
	}
	defer result.Body.Close()

	scanner := bufio.NewScanner(result.Body)

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

		var (
			id           int
			lat, long    float64
			nbBikes      int
			nbStdBikes   int
			nbEBikes     int
			nbEmptyDocks int
			nbDocks      int
		)

		fmt.Sscanf(fields[1], "%d", &id)
		fmt.Sscanf(fields[3], "%f", &lat)
		fmt.Sscanf(fields[4], "%f", &long)
		fmt.Sscanf(fields[5], "%d", &nbBikes)
		fmt.Sscanf(fields[6], "%d", &nbStdBikes)
		fmt.Sscanf(fields[7], "%d", &nbEBikes)
		fmt.Sscanf(fields[8], "%d", &nbEmptyDocks)
		fmt.Sscanf(fields[9], "%d", &nbDocks)

		stations = append(stations, tfl.Station{
			ID:              id,
			Name:            fields[2],
			Lat:             lat,
			Long:            long,
			NbBikes:         nbBikes,
			NbStandardBikes: nbStdBikes,
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

// DeleteSnapshot deletes a specific snapshot from R2.
func (r *R2Storage) DeleteSnapshot(ctx context.Context, key string) error {
	_, err := r.client.DeleteObject(ctx, &s3.DeleteObjectInput{
		Bucket: aws.String(r.bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		return fmt.Errorf("failed to delete object: %w", err)
	}
	return nil
}

// PutObject is a generic method to upload any object to R2
func (r *R2Storage) PutObject(ctx context.Context, key string, data []byte, contentType string) error {
	_, err := r.client.PutObject(ctx, &s3.PutObjectInput{
		Bucket:      aws.String(r.bucket),
		Key:         aws.String(key),
		Body:        bytes.NewReader(data),
		ContentType: aws.String(contentType),
	})
	if err != nil {
		return fmt.Errorf("failed to upload object: %w", err)
	}
	return nil
}

// GetObjectACL retrieves the ACL of an object
func (r *R2Storage) GetObject(ctx context.Context, key string) ([]byte, error) {
	result, err := r.client.GetObject(ctx, &s3.GetObjectInput{
		Bucket: aws.String(r.bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		return nil, fmt.Errorf("failed to get object: %w", err)
	}
	defer result.Body.Close()

	var buf bytes.Buffer
	_, err = buf.ReadFrom(result.Body)
	if err != nil {
		return nil, fmt.Errorf("failed to read object: %w", err)
	}

	return buf.Bytes(), nil
}

// BucketExists checks if the bucket exists
func (r *R2Storage) BucketExists(ctx context.Context) (bool, error) {
	_, err := r.client.HeadBucket(ctx, &s3.HeadBucketInput{
		Bucket: aws.String(r.bucket),
	})
	if err == nil {
		return true, nil
	}

	// Return error with more details for debugging
	return false, fmt.Errorf("failed to access bucket '%s': %w", r.bucket, err)
}
