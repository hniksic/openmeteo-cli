use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use chrono_tz::Tz;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Return severity score for WMO weather code. Higher values indicate more significant weather
/// that should take precedence when aggregating multiple hours.
fn wmo_severity(code: i32) -> i32 {
    match code {
        95..=99 => 100, // Thunderstorm
        80..=86 => 80,  // Rain/snow showers
        71..=77 => 70,  // Snow
        51..=67 => 60,  // Drizzle/Rain
        45 | 48 => 50,  // Fog
        3 => 30,        // Overcast
        2 => 20,        // Partly cloudy
        1 => 10,        // Mainly clear
        0 => 0,         // Clear
        _ => 0,
    }
}

#[derive(Debug, Clone)]
pub struct WeatherPoint {
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
    pub by_model: Vec<(String, Vec<WeatherPoint>)>,
    pub timezone: Tz,
    pub location: Coord,
}

impl Forecast {
    pub fn download(latitude: f64, longitude: f64, models: &[&str]) -> anyhow::Result<Self> {
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

                let forecast: Vec<WeatherPoint> = temps
                    .into_iter()
                    .zip(precips)
                    .zip(codes)
                    .map(|((temp, precip), code)| WeatherPoint { temp, precip, code })
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

    /// Compress forecast data: keep hourly for today, use 3-hour intervals for other days.
    ///
    /// For compressed intervals, temperature is averaged, precipitation is summed, and the most
    /// significant WMO weather code is selected (e.g., rain takes precedence over sun).
    pub fn compress(&mut self, today: NaiveDate) {
        let mut new_times = Vec::new();
        let mut new_by_model: Vec<(String, Vec<WeatherPoint>)> = self
            .by_model
            .iter()
            .map(|(name, _)| (name.clone(), Vec::new()))
            .collect();

        let mut i = 0;
        while i < self.times.len() {
            let time = self.times[i];
            let date = time.date_naive();

            if date == today {
                // Keep hourly for today
                new_times.push(time);
                for (model_idx, (_, weather)) in self.by_model.iter().enumerate() {
                    new_by_model[model_idx].1.push(weather[i].clone());
                }
                i += 1;
            } else {
                // Compress to 3-hour intervals for other days
                let bucket_start_hour = time.hour() / 3 * 3;
                let bucket_end_hour = bucket_start_hour + 3;

                // Find all hours in this 3-hour bucket
                let mut bucket_indices = vec![i];
                let mut j = i + 1;
                while j < self.times.len() {
                    let next_time = self.times[j];
                    if next_time.date_naive() == date && next_time.hour() < bucket_end_hour {
                        bucket_indices.push(j);
                        j += 1;
                    } else {
                        break;
                    }
                }

                // Use the first time in the bucket as the representative time
                new_times.push(time);

                // Aggregate weather for each model
                for (model_idx, (_, weather)) in self.by_model.iter().enumerate() {
                    let points: Vec<&WeatherPoint> =
                        bucket_indices.iter().map(|&idx| &weather[idx]).collect();

                    // Average temperature
                    let temps: Vec<f64> = points.iter().filter_map(|p| p.temp).collect();
                    let avg_temp = if temps.is_empty() {
                        None
                    } else {
                        Some(temps.iter().sum::<f64>() / temps.len() as f64)
                    };

                    // Sum precipitation
                    let precips: Vec<f64> = points.iter().filter_map(|p| p.precip).collect();
                    let sum_precip = if precips.is_empty() {
                        None
                    } else {
                        Some(precips.iter().sum::<f64>())
                    };

                    // Most significant WMO code
                    let most_significant_code = points
                        .iter()
                        .filter_map(|p| p.code)
                        .max_by_key(|&code| wmo_severity(code));

                    new_by_model[model_idx].1.push(WeatherPoint {
                        temp: avg_temp,
                        precip: sum_precip,
                        code: most_significant_code,
                    });
                }

                i = j;
            }
        }

        self.times = new_times;
        self.by_model = new_by_model;
    }
}

#[derive(Debug)]
pub struct Current {
    pub weather: WeatherPoint,
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

        let weather = WeatherPoint {
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
