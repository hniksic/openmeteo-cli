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

/// Resolve a location string to geographic coordinates.
///
/// Accepts either a coordinate pair (e.g., "45.8150,15.9819") or a place name
/// (e.g., "London"). Coordinates are validated to be within valid ranges.
/// Place names are resolved using the Nominatim geocoding API.
pub fn resolve_location(s: &str) -> anyhow::Result<Location> {
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

    // Try parsing as coordinates first
    if let Some(caps) = COORD_RE.captures(s) {
        const NUM_ERR: &str = "Latitude must be between -90 and 90, longitude between -180 and 180";
        let latitude: f64 = caps[1].parse().context(NUM_ERR)?;
        let longitude: f64 = caps[2].parse().context(NUM_ERR)?;

        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            bail!(NUM_ERR);
        }

        return Ok(Location {
            display_name: s.to_string(),
            latitude,
            longitude,
        });
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
