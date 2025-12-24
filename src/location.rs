use std::sync::LazyLock;

use anyhow::{bail, Context};
use regex::Regex;
use serde::Deserialize;

/// A resolved geographic location with coordinates and display name.
#[derive(Debug, Clone)]
pub struct Location {
    /// Human-readable name (place name from Nominatim, or original coordinate string).
    pub display_name: String,
    /// Latitude in degrees, range -90 to 90.
    pub latitude: f64,
    /// Longitude in degrees, range -180 to 180.
    pub longitude: f64,
}

#[derive(Debug, Deserialize)]
struct GeoJsonResponse {
    features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
struct Feature {
    properties: Properties,
    geometry: Geometry,
}

#[derive(Debug, Deserialize)]
struct Properties {
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct Geometry {
    coordinates: [f64; 2], // [lon, lat]
}

/// Parse a coordinate string in "latitude,longitude" format.
///
/// Returns `None` if the string doesn't match the expected format or if
/// coordinates are out of valid ranges (latitude: -90 to 90, longitude: -180 to 180).
fn parse_coordinates(s: &str) -> Option<Location> {
    static COORD_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?x)
            ^
            \s*
            (-?\d+(?:\.\d+)?)   # latitude: decimal number
            \s*,\s*
            (-?\d+(?:\.\d+)?)   # longitude: decimal number
            \s*
            $
        "#,
        )
        .unwrap()
    });

    let caps = COORD_RE.captures(s)?;
    let latitude: f64 = caps[1].parse().ok()?;
    let longitude: f64 = caps[2].parse().ok()?;

    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }

    Some(Location {
        display_name: s.to_string(),
        latitude,
        longitude,
    })
}

/// Resolve a location string to geographic coordinates.
///
/// Accepts either a coordinate pair (e.g., "45.8150,15.9819") or a place name
/// (e.g., "London"). Coordinates are validated to be within valid ranges.
/// Place names are resolved using the Nominatim geocoding API.
pub fn resolve_location(s: &str) -> anyhow::Result<Location> {
    if let Some(location) = parse_coordinates(s) {
        return Ok(location);
    }

    // Use Nominatim for geocoding
    let client = reqwest::blocking::Client::new();
    let response = client
        .get("https://nominatim.openstreetmap.org/search.php")
        .query(&[("q", s), ("format", "geojson")])
        .header("User-Agent", "openmeteo-cli/0.0.1")
        .send()
        .context("Geocoding request failed")?;

    if !response.status().is_success() {
        bail!("Geocoding API error: {}", response.status());
    }

    let mut data: GeoJsonResponse = response.json().context("Geocoding JSON parsing failed")?;
    if data.features.is_empty() {
        bail!("Unknown location");
    }

    let feature = data.features.remove(0);
    let [lon, lat] = feature.geometry.coordinates;
    Ok(Location {
        display_name: feature.properties.display_name,
        latitude: lat,
        longitude: lon,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_coordinates_basic() {
        let loc = parse_coordinates("45.8150,15.9819").unwrap();
        assert_eq!(loc.latitude, 45.8150);
        assert_eq!(loc.longitude, 15.9819);
        assert_eq!(loc.display_name, "45.8150,15.9819");
    }

    #[test]
    fn parse_coordinates_negative() {
        let loc = parse_coordinates("-33.8688,151.2093").unwrap();
        assert_eq!(loc.latitude, -33.8688);
        assert_eq!(loc.longitude, 151.2093);
    }

    #[test]
    fn parse_coordinates_integers() {
        let loc = parse_coordinates("45,15").unwrap();
        assert_eq!(loc.latitude, 45.0);
        assert_eq!(loc.longitude, 15.0);
    }

    #[test]
    fn parse_coordinates_with_whitespace() {
        let loc = parse_coordinates("  45.0 , 15.0  ").unwrap();
        assert_eq!(loc.latitude, 45.0);
        assert_eq!(loc.longitude, 15.0);
    }

    #[test]
    fn parse_coordinates_boundary_values() {
        assert!(parse_coordinates("90,180").is_some());
        assert!(parse_coordinates("-90,-180").is_some());
    }

    #[test]
    fn parse_coordinates_latitude_out_of_range() {
        assert!(parse_coordinates("91,0").is_none());
        assert!(parse_coordinates("-91,0").is_none());
    }

    #[test]
    fn parse_coordinates_longitude_out_of_range() {
        assert!(parse_coordinates("0,181").is_none());
        assert!(parse_coordinates("0,-181").is_none());
    }

    #[test]
    fn parse_coordinates_not_coordinates() {
        assert!(parse_coordinates("London").is_none());
        assert!(parse_coordinates("").is_none());
        assert!(parse_coordinates("45").is_none());
        assert!(parse_coordinates("45,15,20").is_none());
    }
}
