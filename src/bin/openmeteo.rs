use chrono::{DateTime, FixedOffset, Local, Timelike};
use clap::{Parser, Subcommand};
use itertools::Itertools;
use serde::Serialize;

use openmeteo::data::{
    format_precip, format_temp, format_wmo_symbol, Current, Forecast, WeatherPoint,
    MAX_FORECAST_DAYS,
};
use openmeteo::fetch::{download_current, download_forecast};
use openmeteo::location::resolve_location;
use openmeteo::table::Table;
use time::{parse_date_range, resolve_time_range};

#[derive(Parser)]
#[command(name = "openmeteo")]
#[command(about = "Fetch weather data from OpenMeteo")]
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch weather forecast for a given location and dates
    Forecast {
        /// Location name or lat,long pair
        location: String,

        /// Date or range: YYYY-MM-DD, +N, 'today', 'tomorrow', weekday, or date1..date2
        #[arg(default_value = "today", value_name = "DATE_RANGE")]
        dates: String,

        /// Comma-separated list of forecast models
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "ecmwf_ifs,gfs_graphcast025"
        )]
        models: Vec<String>,

        /// Show hourly data for all days (default: 3-hour intervals for future days)
        #[arg(long)]
        full: bool,

        /// Output raw JSON instead of formatted table
        #[arg(long)]
        json: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Fetch current weather for a given location
    Current {
        /// Location name or lat,long pair
        location: String,

        /// Output raw JSON instead of formatted table
        #[arg(long)]
        json: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
}

/// Dedup consecutive identical values, replacing duplicates with empty strings
/// e.g. `dedup(["foo", "foo", "foo", "bar", "bar", "baz"]) == ["foo", "", "", "bar", "", "baz"]`.
fn dedup(items: impl IntoIterator<Item = String>) -> Vec<String> {
    items
        .into_iter()
        .chunk_by(|item| item.clone())
        .into_iter()
        .flat_map(|(key, group)| {
            std::iter::once(key).chain(std::iter::repeat_n(String::new(), group.count() - 1))
        })
        .collect()
}

/// Build a table displaying hourly forecast data for multiple models.
///
/// Filters the forecast data to the requested time range, then constructs a table with Date and
/// Hour columns on the left, followed by weather symbol, temperature, and precipitation columns
/// for each model. Dates are deduped so only the first row of each day shows the date.
fn build_forecast_table(
    time_points: &[DateTime<FixedOffset>],
    by_model: &[(String, Vec<WeatherPoint>)],
    (start_time, end_time): (DateTime<FixedOffset>, DateTime<FixedOffset>),
) -> Table {
    let in_range = |dt| dt >= start_time && dt < end_time;

    let times_in_range = time_points
        .iter()
        .filter(|&&dt| in_range(dt))
        .copied()
        .collect_vec();
    let dates = dedup(
        times_in_range
            .iter()
            .map(|dt| dt.format("%Y-%m-%d").to_string()),
    );
    let hours: Vec<String> = times_in_range
        .iter()
        .map(|dt| dt.format("%Hh").to_string())
        .collect();

    let mut table = Table::new().column("Date", dates).column("Hour", hours);

    for (model, weather_points) in by_model {
        let mut symbols = Vec::new();
        let mut temps = Vec::new();
        let mut precips = Vec::new();

        for (i, &time) in time_points.iter().enumerate() {
            if !in_range(time) {
                continue;
            }
            let weather = weather_points.get(i);
            symbols.push(format_wmo_symbol(
                weather.and_then(|w| w.code),
                time.hour() as u8,
            ));
            temps.push(format_temp(weather.and_then(|w| w.temp)));
            precips.push(format_precip(weather.and_then(|w| w.precip)));
        }

        table = table
            .group(model)
            .column("", symbols)
            .column("Temp", temps)
            .column("Precip", precips);
    }

    table
}

/// Handle the `forecast` subcommand: fetch and display weather forecast.
///
/// Resolves the location (by name or coordinates), parses the date range, downloads forecast data
/// from Open-Meteo for the requested models, and prints the result as a formatted table.
async fn do_forecast(
    location: &str,
    dates: &str,
    models: &[String],
    full: bool,
    json: bool,
    verbose: bool,
) -> anyhow::Result<()> {
    let location = resolve_location(location).await?;
    let date_range = parse_date_range(dates)?;

    let models: Vec<&str> = models.iter().map(|s| s.as_str()).collect();
    let mut forecast = download_forecast(location.latitude, location.longitude, &models).await?;

    let now = Local::now()
        .with_timezone(&forecast.timezone)
        .fixed_offset();

    let time_range = resolve_time_range(date_range, forecast.timezone, now);

    if json {
        print_forecast_json(&forecast, time_range);
        return Ok(());
    }

    println!("Forecast for {}", location.display_name);
    if verbose {
        println!("Grid-cell location: {}", forecast.location.link());
        println!("Timezone: {}", forecast.timezone);
        println!("Interval: [{}, {})", time_range.0, time_range.1);
    }
    if !full {
        forecast.compact(now.date_naive());
    }
    build_forecast_table(&forecast.times, &forecast.by_model, time_range).print();
    Ok(())
}

#[derive(Serialize)]
struct WeatherPointOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    time: DateTime<FixedOffset>,
    latitude: f64,
    longitude: f64,
    temperature: f64,
    precipitation: f64,
    weather_code: u8,
    weather_symbol: &'static str,
}

fn print_forecast_json(
    forecast: &Forecast,
    (start_time, end_time): (DateTime<FixedOffset>, DateTime<FixedOffset>),
) {
    let mut outputs = Vec::new();

    for (model_name, points) in &forecast.by_model {
        for (&time, point) in forecast.times.iter().zip(points) {
            if !(start_time <= time && time < end_time) {
                continue;
            }
            let (Some(temp), Some(precip), Some(code)) = (point.temp, point.precip, point.code)
            else {
                continue;
            };
            outputs.push(WeatherPointOutput {
                model: Some(model_name.clone()),
                time,
                latitude: forecast.location.latitude,
                longitude: forecast.location.longitude,
                temperature: temp,
                precipitation: precip,
                weather_code: code.0,
                weather_symbol: code.raw_symbol(time.hour() as u8),
            });
        }
    }

    outputs.sort_by_key(|o| o.time);
    for output in outputs {
        println!("{}", serde_json::to_string(&output).unwrap());
    }
}

/// Handle the `current` subcommand: fetch and display current weather.
///
/// Resolves the location (by name or coordinates), downloads current weather from Open-Meteo,
/// and prints the result as a single-row table.
async fn do_current(location: &str, json: bool, verbose: bool) -> anyhow::Result<()> {
    let location = resolve_location(location).await?;
    let current = download_current(location.latitude, location.longitude).await?;

    if json {
        print_current_json(&current);
        return Ok(());
    }
    println!("Current weather for {}", location.display_name);
    if verbose {
        println!("Grid-cell location: {}", current.location.link());
    }
    Table::new()
        .column(
            "Time",
            vec![current.time.format("%Y-%m-%d %H:%M").to_string()],
        )
        .column(
            "",
            vec![format_wmo_symbol(
                current.weather.code,
                current.time.hour() as u8,
            )],
        )
        .column("Temp", vec![format_temp(current.weather.temp)])
        .column("Precip", vec![format_precip(current.weather.precip)])
        .print();
    Ok(())
}

fn print_current_json(current: &Current) {
    let (Some(temp), Some(precip), Some(code)) = (
        current.weather.temp,
        current.weather.precip,
        current.weather.code,
    ) else {
        return;
    };
    let output = WeatherPointOutput {
        model: None,
        time: current.time,
        latitude: current.location.latitude,
        longitude: current.location.longitude,
        temperature: temp,
        precipitation: precip,
        weather_code: code.0,
        weather_symbol: code.raw_symbol(current.time.hour() as u8),
    };
    println!("{}", serde_json::to_string(&output).unwrap());
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Forecast {
            location,
            dates,
            models,
            full,
            json,
            verbose,
        } => do_forecast(&location, &dates, &models, full, json, verbose).await,
        Command::Current {
            location,
            json,
            verbose,
        } => do_current(&location, json, verbose).await,
    }
}

mod time {
    use chrono::{
        DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveTime, TimeZone, Weekday,
    };
    use chrono_tz::Tz;

    use super::MAX_FORECAST_DAYS;

    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum RequestedDate {
        Today,
        Tomorrow,
        RelativeDays(u8),
        Weekday(Weekday),
        Absolute(NaiveDate),
    }

    fn parse_weekday(s: &str) -> Option<Weekday> {
        match s {
            "mon" | "monday" => Some(Weekday::Mon),
            "tue" | "tuesday" => Some(Weekday::Tue),
            "wed" | "wednesday" => Some(Weekday::Wed),
            "thu" | "thursday" => Some(Weekday::Thu),
            "fri" | "friday" => Some(Weekday::Fri),
            "sat" | "saturday" => Some(Weekday::Sat),
            "sun" | "sunday" => Some(Weekday::Sun),
            _ => None,
        }
    }

    fn parse_date(s: &str) -> anyhow::Result<RequestedDate> {
        use anyhow::Context;
        let s = s.to_lowercase();
        match s.as_str() {
            "today" => Ok(RequestedDate::Today),
            "tomorrow" => Ok(RequestedDate::Tomorrow),
            _ => {
                if let Some(weekday) = parse_weekday(&s) {
                    Ok(RequestedDate::Weekday(weekday))
                } else if let Some(days) = s.strip_prefix('+').and_then(|n| n.parse::<u8>().ok()) {
                    Ok(RequestedDate::RelativeDays(days))
                } else {
                    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                        .map(RequestedDate::Absolute)
                        .context(
                            "dates must be YYYY-MM-DD, +N, weekday name, 'today' or 'tomorrow'",
                        )
                }
            }
        }
    }

    /// Parse a date string, or return `default` if empty.
    fn parse_date_or(s: &str, default: RequestedDate) -> anyhow::Result<RequestedDate> {
        if s.is_empty() {
            Ok(default)
        } else {
            parse_date(s)
        }
    }

    pub fn parse_date_range(s: &str) -> anyhow::Result<(RequestedDate, RequestedDate)> {
        match s.split_once("..") {
            Some(("", "")) => anyhow::bail!("empty range '..' not allowed"),
            Some((left, right)) => {
                let a = parse_date_or(left, RequestedDate::Today)?;
                let b = parse_date_or(right, RequestedDate::RelativeDays(MAX_FORECAST_DAYS))?;
                Ok((a, b))
            }
            None => {
                let d = parse_date(s)?;
                Ok((d, d))
            }
        }
    }

    fn resolve_date(
        dt: RequestedDate,
        relative_to: NaiveDate,
        weekday_start_at: NaiveDate,
    ) -> NaiveDate {
        match dt {
            RequestedDate::Today => relative_to,
            RequestedDate::Tomorrow => relative_to + Duration::days(1),
            RequestedDate::RelativeDays(n) => relative_to + Duration::days(n.into()),
            RequestedDate::Weekday(wanted) => {
                let mut date = weekday_start_at;
                while date.weekday() != wanted {
                    date += Duration::days(1);
                }
                date
            }
            RequestedDate::Absolute(d) => d,
        }
    }

    /// Convert an inclusive date range to a half-open time interval.
    ///
    /// Input dates are inclusive (e.g., "mon..wed" means Monday through Wednesday).
    /// Output is a half-open interval `[start, end)` suitable for filtering hourly data.
    /// The start time is clamped to `relative_to` to avoid showing past hours.
    pub fn resolve_time_range(
        (start_date, mut end_date): (RequestedDate, RequestedDate),
        timezone: Tz,
        now: DateTime<FixedOffset>,
    ) -> (DateTime<FixedOffset>, DateTime<FixedOffset>) {
        let today = now.date_naive();

        // Open-Meteo provides forecasts at hour starts, so after 23:00 there's no more
        // data for "today". Since start is clamped to `now`, shift end to "tomorrow" to
        // avoid an empty forecast. We use 22:55 as the cutoff to account for network
        // latency.
        const CUTOFF_TIME: NaiveTime = NaiveTime::from_hms_opt(22, 55, 0).unwrap();
        if now.time() > CUTOFF_TIME && end_date == RequestedDate::Today {
            end_date = RequestedDate::Tomorrow;
        }

        let start_resolved = resolve_date(start_date, today, today);
        let end_resolved = resolve_date(end_date, today, start_resolved);

        let start_time = timezone
            .from_local_datetime(&start_resolved.and_time(NaiveTime::MIN))
            .unwrap()
            .fixed_offset();
        let start_time = std::cmp::max(start_time, now);

        let end_resolved = end_resolved + Duration::days(1);
        let end_time = timezone
            .from_local_datetime(&end_resolved.and_time(NaiveTime::MIN))
            .unwrap()
            .fixed_offset();

        (start_time, end_time)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use chrono::{TimeZone, Timelike};

        fn make_time(hour: u32, minute: u32) -> DateTime<FixedOffset> {
            // Use a Wednesday (2025-01-15) as the reference date for weekday tests
            FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2025, 1, 15, hour, minute, 0)
                .unwrap()
        }

        // --- parse_date tests ---

        #[test]
        fn parse_date_today_tomorrow() {
            assert_eq!(parse_date("today").unwrap(), RequestedDate::Today);
            assert_eq!(parse_date("tomorrow").unwrap(), RequestedDate::Tomorrow);
        }

        #[test]
        fn parse_date_case_insensitive() {
            assert_eq!(parse_date("TODAY").unwrap(), RequestedDate::Today);
            assert_eq!(parse_date("Tomorrow").unwrap(), RequestedDate::Tomorrow);
            assert_eq!(
                parse_date("MONDAY").unwrap(),
                RequestedDate::Weekday(Weekday::Mon)
            );
        }

        #[test]
        fn parse_date_weekdays() {
            assert_eq!(
                parse_date("mon").unwrap(),
                RequestedDate::Weekday(Weekday::Mon)
            );
            assert_eq!(
                parse_date("monday").unwrap(),
                RequestedDate::Weekday(Weekday::Mon)
            );
            assert_eq!(
                parse_date("tue").unwrap(),
                RequestedDate::Weekday(Weekday::Tue)
            );
            assert_eq!(
                parse_date("wed").unwrap(),
                RequestedDate::Weekday(Weekday::Wed)
            );
            assert_eq!(
                parse_date("thu").unwrap(),
                RequestedDate::Weekday(Weekday::Thu)
            );
            assert_eq!(
                parse_date("fri").unwrap(),
                RequestedDate::Weekday(Weekday::Fri)
            );
            assert_eq!(
                parse_date("sat").unwrap(),
                RequestedDate::Weekday(Weekday::Sat)
            );
            assert_eq!(
                parse_date("sun").unwrap(),
                RequestedDate::Weekday(Weekday::Sun)
            );
            assert_eq!(
                parse_date("sunday").unwrap(),
                RequestedDate::Weekday(Weekday::Sun)
            );
        }

        #[test]
        fn parse_date_relative_days() {
            assert_eq!(parse_date("+0").unwrap(), RequestedDate::RelativeDays(0));
            assert_eq!(parse_date("+1").unwrap(), RequestedDate::RelativeDays(1));
            assert_eq!(parse_date("+7").unwrap(), RequestedDate::RelativeDays(7));
            assert_eq!(parse_date("+16").unwrap(), RequestedDate::RelativeDays(16));
        }

        #[test]
        fn parse_date_absolute() {
            assert_eq!(
                parse_date("2025-01-15").unwrap(),
                RequestedDate::Absolute(NaiveDate::from_ymd_opt(2025, 1, 15).unwrap())
            );
            assert_eq!(
                parse_date("2024-12-31").unwrap(),
                RequestedDate::Absolute(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap())
            );
        }

        #[test]
        fn parse_date_invalid() {
            assert!(parse_date("").is_err());
            assert!(parse_date("yesterday").is_err());
            assert!(parse_date("15-01-2025").is_err()); // wrong order
            assert!(parse_date("2025/01/15").is_err()); // wrong separator
            assert!(parse_date("invalid").is_err());
        }

        // --- parse_date_range tests ---

        #[test]
        fn parse_date_range_single() {
            let (start, end) = parse_date_range("today").unwrap();
            assert_eq!(start, RequestedDate::Today);
            assert_eq!(end, RequestedDate::Today);
        }

        #[test]
        fn parse_date_range_range() {
            let (start, end) = parse_date_range("today..tomorrow").unwrap();
            assert_eq!(start, RequestedDate::Today);
            assert_eq!(end, RequestedDate::Tomorrow);

            let (start, end) = parse_date_range("mon..fri").unwrap();
            assert_eq!(start, RequestedDate::Weekday(Weekday::Mon));
            assert_eq!(end, RequestedDate::Weekday(Weekday::Fri));

            let (start, end) = parse_date_range("+1..+3").unwrap();
            assert_eq!(start, RequestedDate::RelativeDays(1));
            assert_eq!(end, RequestedDate::RelativeDays(3));
        }

        #[test]
        fn parse_date_range_open_ended() {
            // ..fri means today..fri
            let (start, end) = parse_date_range("..fri").unwrap();
            assert_eq!(start, RequestedDate::Today);
            assert_eq!(end, RequestedDate::Weekday(Weekday::Fri));

            // mon.. means mon..+16
            let (start, end) = parse_date_range("mon..").unwrap();
            assert_eq!(start, RequestedDate::Weekday(Weekday::Mon));
            assert_eq!(end, RequestedDate::RelativeDays(MAX_FORECAST_DAYS));

            // just .. is forbidden
            assert!(parse_date_range("..").is_err());
        }

        #[test]
        fn parse_date_range_invalid() {
            assert!(parse_date_range("invalid..today").is_err());
            assert!(parse_date_range("today..invalid").is_err());
        }

        // --- resolve_time_range tests ---

        /// Test helper that parses a date range string and resolves it in UTC.
        fn test_resolve(
            dates: &str,
            relative_to: DateTime<FixedOffset>,
        ) -> (DateTime<FixedOffset>, DateTime<FixedOffset>) {
            let date_range = parse_date_range(dates).unwrap();
            resolve_time_range(date_range, chrono_tz::UTC, relative_to)
        }

        #[test]
        fn resolve_time_range_today_before_cutoff() {
            let relative_to = make_time(12, 0); // noon
            let (start, end) = test_resolve("today", relative_to);
            // Start should be clamped to relative_to (noon)
            assert_eq!(start.hour(), 12);
            // End should be midnight of the next day
            assert_eq!(
                end.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 16).unwrap()
            );
            assert_eq!(end.hour(), 0);
        }

        #[test]
        fn resolve_time_range_today_after_cutoff() {
            let now = make_time(23, 0); // after 22:55
            let (start, end) = test_resolve("today", now);
            // Start is clamped to now (23:00 today)
            assert_eq!(
                start.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 15).unwrap()
            );
            assert_eq!(start.hour(), 23);
            // End shifts to tomorrow, so end time is midnight day-after-tomorrow
            assert_eq!(
                end.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 17).unwrap()
            );
        }

        #[test]
        fn resolve_time_range_at_cutoff_boundary() {
            // Exactly at 22:55 should NOT trigger the end shift (we use >)
            let (start, end) = test_resolve("today", make_time(22, 55));
            assert_eq!(
                start.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 15).unwrap()
            );
            assert_eq!(
                end.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 16).unwrap()
            );

            // One minute later should trigger the end shift
            let (start, end) = test_resolve("today", make_time(22, 56));
            assert_eq!(
                start.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 15).unwrap()
            );
            assert_eq!(
                end.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 17).unwrap()
            );
        }

        #[test]
        fn resolve_time_range_relative_days() {
            let (start, end) = test_resolve("+2..+3", make_time(10, 0));
            // +2 from 2025-01-15 is 2025-01-17
            assert_eq!(
                start.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 17).unwrap()
            );
            // +3 from 2025-01-15 is 2025-01-18, end is midnight of next day
            assert_eq!(
                end.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 19).unwrap()
            );
        }

        #[test]
        fn resolve_time_range_weekday() {
            // Reference is Wednesday 2025-01-15
            let (start, end) = test_resolve("fri..sun", make_time(10, 0));
            // Friday after Wednesday 2025-01-15 is 2025-01-17
            assert_eq!(
                start.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 17).unwrap()
            );
            // Sunday after Friday is 2025-01-19, end is midnight of next day
            assert_eq!(
                end.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 20).unwrap()
            );
        }

        #[test]
        fn resolve_time_range_absolute_ignores_cutoff() {
            let relative_to = make_time(23, 30); // after cutoff
            let (start, end) = test_resolve("2025-01-15", relative_to);
            // Absolute dates should not be affected by the cutoff
            // But start is still clamped to relative_to
            assert_eq!(start.hour(), 23);
            assert_eq!(start.minute(), 30);
            assert_eq!(
                end.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 16).unwrap()
            );
        }

        #[test]
        fn resolve_time_range_start_clamped_to_relative_to() {
            // If relative_to is in the afternoon, start should be clamped
            let (start, _) = test_resolve("today", make_time(15, 30));
            assert_eq!(start.hour(), 15);
            assert_eq!(start.minute(), 30);
        }

        #[test]
        fn resolve_time_range_respects_timezone() {
            // 10:00 UTC on 2025-01-15
            let relative_to = FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2025, 1, 15, 10, 0, 0)
                .unwrap();

            // In UTC, "tomorrow" starts at 2025-01-16 00:00:00 UTC
            let (start_utc, _) = resolve_time_range(
                parse_date_range("tomorrow").unwrap(),
                chrono_tz::UTC,
                relative_to,
            );
            assert_eq!(
                start_utc.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 16).unwrap()
            );
            assert_eq!(start_utc.hour(), 0);
            assert_eq!(start_utc.offset().local_minus_utc(), 0);

            // In Europe/Zagreb (UTC+1 in winter), "tomorrow" starts at 2025-01-16 00:00:00
            // local, which is 2025-01-15 23:00:00 UTC
            let (start_zagreb, _) = resolve_time_range(
                parse_date_range("tomorrow").unwrap(),
                chrono_tz::Europe::Zagreb,
                relative_to,
            );
            assert_eq!(
                start_zagreb.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 16).unwrap()
            );
            assert_eq!(start_zagreb.hour(), 0);
            assert_eq!(start_zagreb.offset().local_minus_utc(), 3600); // UTC+1

            // The Zagreb time should be 1 hour earlier in absolute terms
            assert_eq!(start_zagreb.timestamp(), start_utc.timestamp() - 3600);
        }
    }
}
