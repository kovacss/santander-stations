package tfl

import "encoding/xml"

// Stations represents the root XML element containing all bike stations.
type Stations struct {
	XMLName    xml.Name  `xml:"stations"`
	LastUpdate int64     `xml:"lastUpdate,attr"`
	Version    string    `xml:"version,attr"`
	Stations   []Station `xml:"station"`
}

// Station represents a single Santander Cycles docking station.
type Station struct {
	ID              int     `xml:"id"`
	Name            string  `xml:"name"`
	TerminalName    string  `xml:"terminalName"`
	Lat             float64 `xml:"lat"`
	Long            float64 `xml:"long"`
	Installed       bool    `xml:"installed"`
	Locked          bool    `xml:"locked"`
	InstallDate     int64   `xml:"installDate"`
	RemovalDate     string  `xml:"removalDate"`
	Temporary       bool    `xml:"temporary"`
	NbBikes         int     `xml:"nbBikes"`
	NbStandardBikes int     `xml:"nbStandardBikes"`
	NbEBikes        int     `xml:"nbEBikes"`
	NbEmptyDocks    int     `xml:"nbEmptyDocks"`
	NbDocks         int     `xml:"nbDocks"`
}
