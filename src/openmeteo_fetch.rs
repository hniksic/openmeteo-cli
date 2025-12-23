use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

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
    pub times: Vec<DateTime<FixedOffset>>,
    pub by_model: Vec<(String, Vec<Weather>)>,
    pub timezone: Tz,
    pub location: Coord,
}

impl Forecast {
    pub fn download(latitude: f64, longitude: f64, models: &[String]) -> anyhow::Result<Self> {
        #[derive(Debug, Deserialize)]
        struct Response {
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

        let client = reqwest::blocking::Client::new();
        let models_str = models.join(",");

        let response = client
            .get("https://api.open-meteo.com/v1/forecast")
            .query(&Query {
                latitude,
                longitude,
                hourly: "temperature_2m,precipitation,weather_code",
                models: &models_str,
                forecast_days: 16,
                timezone: "auto",
            })
            .send()
            .context("HTTP request failed")?;

        if !response.status().is_success() {
            bail!("API error: {}", response.status());
        }

        let mut data: Response = response.json().context("JSON parsing failed")?;

        let times: Vec<DateTime<FixedOffset>> = data
            .hourly
            .time
            .iter()
            .map(|t| {
                let naive = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M")
                    .expect("Failed to parse time");
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
                    .take_field_array::<i32>(&propname("weather_code", model));

                let forecast: Vec<Weather> = temps
                    .into_iter()
                    .zip(precips)
                    .zip(codes)
                    .map(|((temp, precip), code)| Weather { temp, precip, code })
                    .collect();

                (model.clone(), forecast)
            })
            .collect();

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
    pub time: DateTime<FixedOffset>,
    pub location: Coord,
}

impl Current {
    pub fn download(latitude: f64, longitude: f64) -> anyhow::Result<Self> {
        #[derive(Debug, Deserialize)]
        struct Response {
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

        #[derive(Serialize)]
        struct Query<'a> {
            latitude: f64,
            longitude: f64,
            current: &'a str,
            timezone: &'a str,
        }

        let client = reqwest::blocking::Client::new();

        let response = client
            .get("https://api.open-meteo.com/v1/forecast")
            .query(&Query {
                latitude,
                longitude,
                current: "temperature_2m,precipitation,weather_code",
                timezone: "auto",
            })
            .send()
            .context("HTTP request failed")?;

        if !response.status().is_success() {
            bail!("API error: {}", response.status());
        }

        let data: Response = response.json().context("JSON parsing failed")?;

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
}
