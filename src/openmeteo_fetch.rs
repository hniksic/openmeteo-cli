use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use serde::de::DeserializeOwned;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Weather {
    pub temp: Option<f64>,
    pub precip: Option<f64>,
    pub code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct Coord {
    pub latitude: f64,
    pub longitude: f64,
}

impl Coord {
    pub fn link(&self) -> String {
        format!(
            "https://www.google.com/maps/place/{},{}",
            self.latitude, self.longitude
        )
    }
}

#[derive(Debug)]
pub struct Forecast {
    pub times: Vec<chrono::DateTime<Tz>>,
    pub by_model: Vec<(String, Vec<Weather>)>,
    pub timezone: Tz,
    pub location: Coord,
}

impl Forecast {
    pub fn download(latitude: f64, longitude: f64, models: &[String]) -> anyhow::Result<Self> {
        #[derive(Debug, Deserialize)]
        struct ForecastResponse {
            latitude: f64,
            longitude: f64,
            timezone: Tz,
            hourly: HourlyData,
        }

        #[derive(Debug, Deserialize)]
        struct HourlyData {
            time: Vec<String>,
            #[serde(flatten)]
            data: HashMap<String, Vec<serde_json::Value>>,
        }

        impl HourlyData {
            fn take_field_array<T: DeserializeOwned>(&mut self, key: &str) -> Vec<Option<T>> {
                self.data
                    .remove(key)
                    .and_then(|v| serde_json::from_value(serde_json::Value::Array(v)).ok())
                    .unwrap_or_default()
            }
        }

        let client = reqwest::blocking::Client::new();
        let models_str = models.join(",");

        let response = client
            .get("https://api.open-meteo.com/v1/forecast")
            .query(&[
                ("latitude", latitude.to_string().as_str()),
                ("longitude", &longitude.to_string()),
                ("hourly", "temperature_2m,precipitation,weather_code"),
                ("models", &models_str),
                ("forecast_days", "16"),
                ("timezone", "auto"),
            ])
            .send()
            .context("HTTP request failed")?;

        if !response.status().is_success() {
            bail!("API error: {}", response.status());
        }

        let mut data: ForecastResponse = response.json().context("JSON parsing failed")?;

        let times: Vec<chrono::DateTime<Tz>> = data
            .hourly
            .time
            .iter()
            .map(|t| {
                let naive = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M")
                    .expect("Failed to parse time");
                data.timezone.from_local_datetime(&naive).unwrap()
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

        let mut by_model = Vec::new();
        for model in models {
            let temps = data
                .hourly
                .take_field_array::<f64>(&propname("temperature_2m", model));
            let precips = data
                .hourly
                .take_field_array::<f64>(&propname("precipitation", model));
            let codes = data
                .hourly
                .take_field_array::<i32>(&propname("weather_code", model));

            let forecast: Vec<Weather> = temps
                .into_iter()
                .zip(precips)
                .zip(codes)
                .map(|((temp, precip), code)| Weather { temp, precip, code })
                .collect();

            by_model.push((model.clone(), forecast));
        }

        Ok(Forecast {
            times,
            by_model,
            timezone: data.timezone,
            location,
        })
    }
}

#[derive(Debug)]
pub struct Current {
    pub weather: Weather,
    pub time: chrono::DateTime<Tz>,
    pub location: Coord,
}

pub fn download_current(latitude: f64, longitude: f64) -> anyhow::Result<Current> {
    #[derive(Debug, Deserialize)]
    struct CurrentResponse {
        latitude: f64,
        longitude: f64,
        timezone: Tz,
        current: CurrentData,
    }

    #[derive(Debug, Deserialize)]
    struct CurrentData {
        time: String,
        temperature_2m: Option<f64>,
        precipitation: Option<f64>,
        weather_code: Option<i32>,
    }

    let client = reqwest::blocking::Client::new();

    let response = client
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[
            ("latitude", latitude.to_string()),
            ("longitude", longitude.to_string()),
            (
                "current",
                "temperature_2m,precipitation,weather_code".to_string(),
            ),
            ("timezone", "auto".to_string()),
        ])
        .send()
        .context("HTTP request failed")?;

    if !response.status().is_success() {
        bail!("API error: {}", response.status());
    }

    let data: CurrentResponse = response.json().context("JSON parsing failed")?;

    let naive = NaiveDateTime::parse_from_str(&data.current.time, "%Y-%m-%dT%H:%M")
        .context("Failed to parse time")?;
    let time = data.timezone.from_local_datetime(&naive).unwrap();

    let location = Coord {
        latitude: data.latitude,
        longitude: data.longitude,
    };

    let weather = Weather {
        temp: data.current.temperature_2m,
        precip: data.current.precipitation,
        code: data.current.weather_code,
    };

    Ok(Current {
        weather,
        time,
        location,
    })
}
