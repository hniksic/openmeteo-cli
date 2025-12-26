use chrono::{DateTime, FixedOffset, NaiveDate, Timelike};
use unicode_width::UnicodeWidthStr;

/// Maximum forecast days supported by Open-Meteo.
pub const MAX_FORECAST_DAYS: u8 = 16;

/// WMO weather code with display formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WmoCode(pub u8);

impl WmoCode {
    /// Return severity score for WMO weather code. Higher values indicate more significant weather
    /// that should take precedence when aggregating multiple hours.
    pub fn severity(self) -> u8 {
        match self.0 {
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

    /// Return weather emoji for this WMO code at the given hour.
    pub fn raw_symbol(self, hour: u8) -> &'static str {
        let is_night = !(6..20).contains(&hour);
        match self.0 {
            0 if is_night => "\u{1F319}",       // CRESCENT MOON - Clear sky (night)
            0 => "\u{1F31E}",                   // BLACK SUN WITH RAYS - Clear sky
            1 if is_night => "\u{1F319}",       // CRESCENT MOON - Mainly clear (night)
            1 => "\u{1F324}",                   // WHITE SUN WITH SMALL CLOUD - Mainly clear
            2 if is_night => "\u{2601}",        // CLOUD - Partly cloudy (night)
            2 => "\u{26C5}",                    // SUN BEHIND CLOUD - Partly cloudy
            3 => "\u{2601}",                    // CLOUD - Overcast
            45 | 48 => "\u{1F32B}",             // FOG
            51..=67 => "\u{1F327}",             // CLOUD WITH RAIN - Drizzle/Rain
            71..=75 => "\u{2744}",              // SNOWFLAKE - Snow
            77 | 85 | 86 => "\u{1F328}",        // CLOUD WITH SNOW - Snow grains/showers
            80..=82 if is_night => "\u{1F327}", // CLOUD WITH RAIN - Rain showers (night)
            80..=82 => "\u{1F326}",             // WHITE SUN BEHIND CLOUD WITH RAIN - Rain showers
            95..=99 => "\u{26C8}",              // THUNDER CLOUD AND RAIN - Thunderstorm
            _ => "?",
        }
    }

    /// Return weather emoji for display in a table column.
    ///
    /// Weather emoji have inconsistent grapheme widths (1 or 2). To align columns, we add a space
    /// after narrow (width 1) emoji, normalizing all symbols to width 2. This must happen here
    /// rather than in generic padding because the space must immediately follow the emoji.
    pub fn symbol(self, hour: u8) -> String {
        let sym = self.raw_symbol(hour);
        if sym.width() == 1 {
            format!("{} ", sym)
        } else {
            sym.to_string()
        }
    }
}

/// Format an optional WMO code as a weather symbol.
pub fn format_wmo_symbol(code: Option<WmoCode>, hour: u8) -> String {
    match code {
        None => "-".to_string(),
        Some(c) => c.symbol(hour),
    }
}

/// Format an optional temperature value.
pub fn format_temp(temp: Option<f64>) -> String {
    match temp {
        // as i32 so -0.1 doesn't show up as -0
        Some(t) => format!("{}°", t.round() as i32),
        None => "-".to_string(),
    }
}

/// Format an optional precipitation value.
pub fn format_precip(precip: Option<f64>) -> String {
    match precip {
        Some(0.0) => String::new(),
        Some(p) if p < 5. => format!("{p:.1}mm"),
        Some(p) => format!("{p:.0}mm"),
        None => "-".to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct WeatherPoint {
    pub temp: Option<f64>,
    pub precip: Option<f64>,
    pub code: Option<WmoCode>,
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
    pub timezone: chrono_tz::Tz,
    pub location: Coord,
}

impl Forecast {
    /// Compact forecast data into a smaller number of points: keep hourly for today, use
    /// 3-hour intervals for other days.
    ///
    /// For compacted intervals, temperature is averaged, precipitation is summed, and the
    /// most significant WMO weather code is selected (e.g., rain takes precedence over
    /// sun).
    pub fn compact(&mut self, today: NaiveDate) {
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
                        .max_by_key(|code| code.severity());

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
