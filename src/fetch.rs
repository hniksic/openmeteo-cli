use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::{NaiveDateTime, TimeZone};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::data::{Coord, Current, Forecast, WeatherPoint, WmoCode, MAX_FORECAST_DAYS};

/// Download weather forecast from Open-Meteo API.
pub async fn download_forecast(
    latitude: f64,
    longitude: f64,
    models: &[&str],
) -> anyhow::Result<Forecast> {
    #[derive(Debug, Deserialize)]
    struct Response {
        latitude: f64,
        longitude: f64,
        timezone: chrono_tz::Tz,
        hourly: HourlyData,
    }

    #[derive(Debug, Deserialize)]
    struct HourlyData {
        time: Vec<String>,
        #[serde(flatten)]
        data: HashMap<String, Vec<serde_json::Value>>,
    }

    impl HourlyData {
        /// Remove `key` from data and deserialize its JSON array into `Vec<Option<T>>`.
        fn take_field_array<T: DeserializeOwned>(&mut self, key: &str) -> Vec<Option<T>> {
            self.data
                .remove(key)
                .and_then(|v| serde_json::from_value(serde_json::Value::Array(v)).ok())
                .unwrap_or_default()
        }
    }

    #[derive(Serialize)]
    struct Query<'a> {
        latitude: f64,
        longitude: f64,
        hourly: &'a str,
        models: &'a str,
        forecast_days: u8,
        timezone: &'a str,
    }

    let client = reqwest::Client::new();
    let models_str = models.join(",");

    let response = client
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&Query {
            latitude,
            longitude,
            hourly: "temperature_2m,precipitation,weather_code",
            models: &models_str,
            forecast_days: MAX_FORECAST_DAYS,
            timezone: "auto",
        })
        .send()
        .await
        .context("HTTP request failed")?;

    if !response.status().is_success() {
        bail!("API error: {}", response.status());
    }

    let mut data: Response = response.json().await.context("JSON parsing failed")?;

    let times = data
        .hourly
        .time
        .iter()
        .map(|t| {
            let naive =
                NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M").expect("Failed to parse time");
            data.timezone
                .from_local_datetime(&naive)
                .unwrap()
                .fixed_offset()
        })
        .collect();

    let location = Coord {
        latitude: data.latitude,
        longitude: data.longitude,
    };

    let propname = |prop: &str, model: &str| -> String {
        if models.len() == 1 {
            prop.to_string()
        } else {
            format!("{}_{}", prop, model)
        }
    };

    let by_model = models
        .iter()
        .map(|model| {
            let temps = data
                .hourly
                .take_field_array::<f64>(&propname("temperature_2m", model));
            let precips = data
                .hourly
                .take_field_array::<f64>(&propname("precipitation", model));
            let codes = data
                .hourly
                .take_field_array::<u8>(&propname("weather_code", model));

            let forecast: Vec<WeatherPoint> = temps
                .into_iter()
                .zip(precips)
                .zip(codes)
                .map(|((temp, precip), code)| WeatherPoint {
                    temp,
                    precip,
                    code: code.map(WmoCode),
                })
                .collect();

            (model.to_string(), forecast)
        })
        .collect();

    Ok(Forecast {
        times,
        by_model,
        timezone: data.timezone,
        location,
    })
}

/// Download current weather from Open-Meteo API.
pub async fn download_current(latitude: f64, longitude: f64) -> anyhow::Result<Current> {
    #[derive(Debug, Deserialize)]
    struct Response {
        latitude: f64,
        longitude: f64,
        timezone: chrono_tz::Tz,
        current: CurrentData,
    }

    #[derive(Debug, Deserialize)]
    struct CurrentData {
        time: String,
        temperature_2m: Option<f64>,
        precipitation: Option<f64>,
        weather_code: Option<u8>,
    }

    #[derive(Serialize)]
    struct Query<'a> {
        latitude: f64,
        longitude: f64,
        current: &'a str,
        timezone: &'a str,
    }

    let client = reqwest::Client::new();

    let response = client
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&Query {
            latitude,
            longitude,
            current: "temperature_2m,precipitation,weather_code",
            timezone: "auto",
        })
        .send()
        .await
        .context("HTTP request failed")?;

    if !response.status().is_success() {
        bail!("API error: {}", response.status());
    }

    let data: Response = response.json().await.context("JSON parsing failed")?;

    let naive = NaiveDateTime::parse_from_str(&data.current.time, "%Y-%m-%dT%H:%M")
        .context("Failed to parse time")?;
    let time = data
        .timezone
        .from_local_datetime(&naive)
        .unwrap()
        .fixed_offset();

    let location = Coord {
        latitude: data.latitude,
        longitude: data.longitude,
    };

    let weather = WeatherPoint {
        temp: data.current.temperature_2m,
        precip: data.current.precipitation,
        code: data.current.weather_code.map(WmoCode),
    };

    Ok(Current {
        weather,
        time,
        location,
    })
}
