# Roadmap: Making This Feel Like a Product

Three initiatives to turn this from "a live map of TfL stations" into
something people would actually choose to open. Each is scoped against the
current codebase, not written in the abstract.

## 1. Trip Preview — nearest available bikes between two points

The core feature people actually want: "I'm at A, going to B — where do I
grab a bike, and where do I drop it off?"

**Status: base version shipped.** Tap "Plan a Trip", tap two points on the
map (either empty space or directly on a station — both set the point at
the exact location clicked), and it highlights the nearest station with a
bike near the start and the nearest station with a free dock near the end,
draws the walk/ride/walk route, and shows distances + a rough time
estimate. Client-side only, no new backend endpoint. Verified on both
desktop and a 375px mobile viewport (touch taps, panel layout, no
overlap/scroll issues).

- [x] **Decide the MVP shape**: compute nearest stations client-side, in the
  browser, against the data `/api/stations` already returns (zero backend
  changes, ~800 stations is nothing to scan) vs. a dedicated
  `/api/nearest?lat=&lng=&limit=` endpoint. Start client-side; only move
  server-side if station counts grow (multi-city) or geocoding moves there.
- [x] Add two location inputs to the map UI — "Start" and "End" — via
  click-on-map (shipped: click/tap-on-map only)
- [ ] Free-text address search: geocode via Nominatim (OpenStreetMap, no
  API key needed) — there's a maintained Leaflet plugin for this. Not
  built yet; click-on-map covers the base case.
- [ ] "Use my location" (`navigator.geolocation`) as a third way to set the
  start point
- [x] Haversine distance function (pure JS) to rank stations by proximity;
  filter the origin candidates to `nbBikes > 0` and the destination
  candidates to `nbEmptyDocks > 0`
- [x] Render origin/destination pins plus the nearest pickup/dropoff
  station, visually distinct from the rest of the map (highlighted ring +
  dashed/solid route lines)
- [x] Info panel: distance to the pickup station, distance from the dropoff
  station to the actual destination, straight-line trip distance, rough
  walk/cycle time estimate
- [x] Handle the empty case: no bikes/docks found near a point shows a clear
  error message rather than nothing (not yet progressively widening the
  search radius — just reports no candidates found)
- [ ] *(Stretch)* real cycling directions instead of straight lines, via a
  routing API (OSRM's public demo server is free and good enough to start)
- [ ] *(Follow-up)* only the single nearest station is considered per side;
  showing the top 2-3 candidates (e.g. if the nearest has just 1 bike) would
  be a small, worthwhile extension

## 2. A Small Cycling-Themed Visual Identity

Right now it's a Leaflet default map with colored dots. Doesn't need a full
redesign — a handful of consistent touches go a long way.

- [ ] Pick one small, cohesive palette and apply it consistently (keep the
  existing green/yellow/red availability semantics — those are meaningful,
  don't touch them — but unify the chrome: header, panels, buttons)
- [ ] Custom favicon (a simple bike glyph) replacing the browser default
- [ ] Swap the plain colored `circleMarker`s for small bike-icon markers,
  still color-coded by availability (Leaflet `divIcon`/SVG icons make this
  straightforward, no new dependency)
- [ ] A lightweight branded header bar (product name + a small bike mark)
  instead of the map just appearing under a floating stats panel
- [ ] One deliberate web font for headings (system-font fallback is fine for
  body text) instead of the current default stack
- [ ] Pass over the legend/info panel/playback controls so they read as one
  designed widget rather than a collection of separately-styled boxes

## 3. Multi-City UK Support

The biggest lift of the three — this is a real architecture change, not a
feature flag. Worth scoping carefully before starting.

- [ ] **Add a city/scheme dimension to the data model.** Today
  `stations`/`snapshots`/`snapshot_aggregates` are one global flat
  namespace keyed by TfL's own numeric station id — a second city's scheme
  could collide with those ids. Two paths: (a) a `city_id` column +
  composite keys everywhere, or (b) fully separate storage per city (D1
  database or R2 prefix per city — simpler isolation, more infra to
  provision per city added)
- [ ] **Define a provider-adapter boundary**: something that produces a
  `Vec<Station>` for a given city, so each city's data source is a small,
  isolated adapter and the collector/storage/web code stays untouched.
  `cf-worker/src/tfl.rs`'s `fetch_stations() -> Stations` is already
  shaped like this by accident — worth making it the actual interface.
- [ ] **Consider standardizing on GBFS** (General Bikeshare Feed
  Specification) as the internal normalized format. It's the industry
  standard and several UK schemes publish GBFS feeds directly — adopting
  it as the adapter output format (instead of this project's bespoke
  `Station` shape) would mean most future cities need near-zero custom
  parsing.
- [ ] **Candidate next cities** (verify the current operator/API for each
  before building — UK bike-share contracts change hands periodically, so
  treat this list as a starting point, not a commitment):
  - Belfast — Belfast Bikes (Nextbike platform)
  - Glasgow — nextbike Glasgow (Nextbike platform)
  - Edinburgh — check current operator (has moved between operators before)
  - Manchester / Liverpool — check current operator (Beryl Bikes runs
    several UK regional schemes)
  - Oxford / Cambridge / Bath — smaller local docked schemes, lower priority
  - If several target cities run on the same platform (Nextbike shows up
    repeatedly above), one adapter likely covers multiple cities at once
- [ ] City switcher in the UI — URL-based (`/london`, `/glasgow`) is simpler
  to share/bookmark than a dropdown-only selector, worth doing both
- [ ] Per-city collector scheduling — different providers may have
  different rate limits or sensible polling intervals; don't assume every
  city wants TfL's 15-minute cadence
- [ ] Decide collector topology: one cron looping over all configured
  cities vs. one Cron Trigger per city (affects blast radius when one
  city's upstream API has a bad day — see the TfL Managed Challenge saga in
  `cf-worker/README.md` for why that matters)

## Suggested Order

1 and 2 are independent and can happen in either order or in parallel — 1
is the actual value-add, 2 is comparatively cheap polish. 3 is the largest
and most architecturally invasive; doing it last means 1 and 2 get built
against a stable single-city shape first, and the provider-adapter
boundary in 3 can be informed by whatever real decisions get made building
1 (e.g. if trip-preview ends up needing a dedicated `/api/nearest`
endpoint, that's exactly the kind of thing a multi-city data model needs to
account for).
