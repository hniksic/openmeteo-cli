use std::sync::LazyLock;

use anyhow::{bail, Context};
use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Location {
    pub display_name: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Deserialize)]
struct NominatimResult {
    display_name: String,
    lat: String,
    lon: String,
}

pub fn resolve_location(s: &str) -> anyhow::Result<Location> {
    static COORD_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*$").unwrap()
    });

    // Try parsing as coordinates first
    if let Some(caps) = COORD_RE.captures(s) {
        let latitude: f64 = caps[1].parse()?;
        let longitude: f64 = caps[2].parse()?;

        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            bail!("Latitude must be between -90 and 90, longitude between -180 and 180");
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
        .query(&[("q", s), ("format", "jsonv2")])
        .header("User-Agent", "curl/8.9.1")
        .send()
        .context("Geocoding request failed")?;

    if !response.status().is_success() {
        bail!("Geocoding API error: {}", response.status());
    }

    let locations: Vec<NominatimResult> =
        response.json().context("Geocoding JSON parsing failed")?;

    if locations.is_empty() {
        bail!("unknown location {}", s);
    }

    let loc = &locations[0];
    let latitude: f64 = loc.lat.parse().context("Invalid latitude")?;
    let longitude: f64 = loc.lon.parse().context("Invalid longitude")?;

    Ok(Location {
        display_name: loc.display_name.clone(),
        latitude,
        longitude,
    })
}
