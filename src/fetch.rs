use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::TimeZone;
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
        time: Vec<i64>,
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
        timeformat: &'a str,
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
            timeformat: "unixtime",
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
        .map(|&ts| {
            data.timezone
                .timestamp_opt(ts, 0)
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
        time: i64,
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
        timeformat: &'a str,
    }

    let client = reqwest::Client::new();

    let response = client
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&Query {
            latitude,
            longitude,
            current: "temperature_2m,precipitation,weather_code",
            timezone: "auto",
            timeformat: "unixtime",
        })
        .send()
        .await
        .context("HTTP request failed")?;

    if !response.status().is_success() {
        bail!("API error: {}", response.status());
    }

    let data: Response = response.json().await.context("JSON parsing failed")?;

    let time = data
        .timezone
        .timestamp_opt(data.current.time, 0)
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
