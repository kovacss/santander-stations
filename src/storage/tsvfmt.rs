//! The on-disk/on-R2 TSV format, shared by every storage backend.

use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};

use crate::tfl::Station;

/// Column headers for the TSV file.
pub const TSV_HEADER: &str =
    "timestamp\tid\tname\tlat\tlong\tnb_bikes\tnb_standard_bikes\tnb_ebikes\tnb_empty_docks\tnb_docks";

/// Timestamp layout used in snapshot file names: `stations_20260205_144714.tsv`.
pub const FILENAME_TIME_FORMAT: &str = "%Y%m%d_%H%M%S";

/// Builds the file name for a snapshot taken at `timestamp`.
pub fn snapshot_filename(timestamp: DateTime<Utc>) -> String {
    format!(
        "stations_{}.tsv",
        timestamp.format(FILENAME_TIME_FORMAT)
    )
}

/// Extracts the timestamp from a snapshot file name or R2 key.
///
/// Accepts anything containing `stations_YYYYMMDD_HHMMSS`, so it works for both
/// bare file names and prefixed keys such as `snapshots/stations_...tsv`.
pub fn parse_timestamp_from_key(key: &str) -> Option<DateTime<Utc>> {
    const MARKER: &str = "stations_";
    const LEN: usize = 15; // YYYYMMDD_HHMMSS

    let start = key.find(MARKER)? + MARKER.len();
    let raw = key.get(start..start + LEN)?;

    NaiveDateTime::parse_from_str(raw, FILENAME_TIME_FORMAT)
        .ok()
        .map(|naive| naive.and_utc())
}

/// Renders stations as TSV content, including the header row.
pub fn render(stations: &[Station], timestamp: DateTime<Utc>) -> String {
    let ts = timestamp.to_rfc3339_opts(SecondsFormat::Secs, true);

    let mut out = String::with_capacity(TSV_HEADER.len() + stations.len() * 96);
    out.push_str(TSV_HEADER);
    out.push('\n');

    for s in stations {
        out.push_str(&format!(
            "{ts}\t{id}\t{name}\t{lat:.6}\t{long:.6}\t{bikes}\t{standard}\t{ebikes}\t{empty}\t{docks}\n",
            ts = ts,
            id = s.id,
            // Escape tabs in name so the column layout survives.
            name = s.name.replace('\t', " "),
            lat = s.lat,
            long = s.long,
            bikes = s.nb_bikes,
            standard = s.nb_standard_bikes,
            ebikes = s.nb_ebikes,
            empty = s.nb_empty_docks,
            docks = s.nb_docks,
        ));
    }

    out
}

/// Parses TSV content back into stations plus the snapshot timestamp taken from
/// the first data row. Malformed rows are skipped.
pub fn parse(content: &str) -> (Vec<Station>, Option<DateTime<Utc>>) {
    let mut stations = Vec::new();
    let mut timestamp = None;

    // Skip the header row.
    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 10 {
            continue;
        }

        if timestamp.is_none() {
            timestamp = DateTime::parse_from_rfc3339(fields[0])
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
        }

        stations.push(Station {
            id: fields[1].parse().unwrap_or_default(),
            name: fields[2].to_string(),
            lat: fields[3].parse().unwrap_or_default(),
            long: fields[4].parse().unwrap_or_default(),
            nb_bikes: fields[5].parse().unwrap_or_default(),
            nb_standard_bikes: fields[6].parse().unwrap_or_default(),
            nb_ebikes: fields[7].parse().unwrap_or_default(),
            nb_empty_docks: fields[8].parse().unwrap_or_default(),
            nb_docks: fields[9].parse().unwrap_or_default(),
            ..Station::default()
        });
    }

    (stations, timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station() -> Station {
        Station {
            id: 1,
            name: "River Street\t, Clerkenwell".to_string(),
            lat: 51.529163,
            long: -0.109971,
            nb_bikes: 4,
            nb_standard_bikes: 2,
            nb_ebikes: 2,
            nb_empty_docks: 15,
            nb_docks: 19,
            ..Station::default()
        }
    }

    #[test]
    fn round_trips_stations() {
        let ts = parse_timestamp_from_key("snapshots/stations_20260205_144714.tsv").unwrap();
        let content = render(&[station()], ts);

        assert!(content.starts_with(TSV_HEADER));
        assert!(content.contains("2026-02-05T14:47:14Z"));
        // The tab inside the name must not create an extra column.
        assert!(content.contains("River Street , Clerkenwell"));

        let (parsed, parsed_ts) = parse(&content);
        assert_eq!(parsed_ts, Some(ts));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, 1);
        assert_eq!(parsed[0].nb_ebikes, 2);
        assert!((parsed[0].lat - 51.529163).abs() < 1e-9);
    }

    #[test]
    fn builds_and_parses_file_names() {
        let ts = parse_timestamp_from_key("stations_20260205_160245.tsv").unwrap();
        assert_eq!(snapshot_filename(ts), "stations_20260205_160245.tsv");
        assert_eq!(parse_timestamp_from_key("nonsense.tsv"), None);
        assert_eq!(parse_timestamp_from_key("stations_short.tsv"), None);
    }

    #[test]
    fn skips_malformed_rows() {
        let content = format!("{TSV_HEADER}\ntruncated\trow\n");
        let (stations, timestamp) = parse(&content);
        assert!(stations.is_empty());
        assert_eq!(timestamp, None);
    }
}
