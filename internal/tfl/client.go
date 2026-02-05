package tfl

import (
	"encoding/xml"
	"fmt"
	"io"
	"net/http"
	"time"
)

const (
	// DefaultEndpoint is the TFL Santander Cycles live update feed.
	DefaultEndpoint = "https://tfl.gov.uk/tfl/syndication/feeds/cycle-hire/livecyclehireupdates.xml"
	// DefaultTimeout for HTTP requests.
	DefaultTimeout = 30 * time.Second
)

// Client fetches station data from the TFL API.
type Client struct {
	endpoint   string
	httpClient *http.Client
}

// NewClient creates a new TFL client with default settings.
func NewClient() *Client {
	return &Client{
		endpoint: DefaultEndpoint,
		httpClient: &http.Client{
			Timeout: DefaultTimeout,
		},
	}
}

// NewClientWithEndpoint creates a new TFL client with a custom endpoint.
func NewClientWithEndpoint(endpoint string) *Client {
	return &Client{
		endpoint: endpoint,
		httpClient: &http.Client{
			Timeout: DefaultTimeout,
		},
	}
}

// FetchStations retrieves the current station data from the TFL API.
func (c *Client) FetchStations() (*Stations, error) {
	req, err := http.NewRequest("GET", c.endpoint, nil)
	if err != nil {
		return nil, fmt.Errorf("failed to create request: %w", err)
	}
	req.Header.Set("User-Agent", "city-cycling/1.0")

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("failed to fetch stations: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("unexpected status code: %d", resp.StatusCode)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("failed to read response body: %w", err)
	}

	var stations Stations
	if err := xml.Unmarshal(body, &stations); err != nil {
		return nil, fmt.Errorf("failed to parse XML: %w", err)
	}

	return &stations, nil
}
