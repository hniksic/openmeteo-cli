mod location;
mod openmeteo_fetch;
mod table;

use chrono::{DateTime, FixedOffset, Local, Timelike};
use clap::{Parser, Subcommand};

use itertools::Itertools;
use location::resolve_location;
use openmeteo_fetch::{Current, Forecast, WeatherPoint};
use table::Table;
use time::{parse_date_range, resolve_time_range};
use unicode_width::UnicodeWidthStr;

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

        /// YYYY-MM-DD, 'today', 'tomorrow', or weekday, or date1..date2
        #[arg(default_value = "today")]
        dates: String,

        /// Comma-separated list of forecast models
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "gfs_graphcast025,ecmwf_ifs025"
        )]
        models: Vec<String>,

        /// Show hourly data for all days (default: 3-hour intervals for future days)
        #[arg(long)]
        full: bool,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    /// Fetch current weather for a given location
    Current {
        /// Location name or lat,long pair
        location: String,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
}

fn wmo_to_symbol(code: i32, is_night: bool) -> &'static str {
    match code {
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
fn weather_symbol(code: Option<i32>, hour: u32) -> String {
    match code {
        None => "-".to_string(),
        Some(c) => {
            let is_night = !(6..20).contains(&hour);
            let sym = wmo_to_symbol(c, is_night);
            if sym.width() == 1 {
                format!("{} ", sym)
            } else {
                sym.to_string()
            }
        }
    }
}

/// Dedup consecutive identical values, replacing duplicates with empty strings
/// e.g. `dedup(["foo", "foo", "foo", "bar", "bar", "baz"]) == ["foo", "", "", "bar", "", "baz"]`.
pub fn dedup(items: impl IntoIterator<Item = String>) -> Vec<String> {
    items
        .into_iter()
        .chunk_by(|item| item.clone())
        .into_iter()
        .flat_map(|(key, group)| {
            std::iter::once(key).chain(std::iter::repeat_n(String::new(), group.count() - 1))
        })
        .collect()
}

fn format_precip(precip: Option<f64>) -> String {
    match precip {
        Some(0.0) => String::new(),
        Some(p) if p < 5. => format!("{p:.1}mm"),
        Some(p) => format!("{p:.0}mm"),
        None => "-".to_string(),
    }
}

fn format_temp(temp: Option<f64>) -> String {
    match temp {
        // as i32 so -0.1 doesn't show up as -0
        Some(t) => format!("{}°", t.round() as i32),
        None => "-".to_string(),
    }
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
    let num_rows = time_points.len();

    let mut table = Table::new().column("Date", dates).column("Hour", hours);

    for (model, weather_points) in by_model {
        let mut temps = Vec::new();
        let mut precips = Vec::new();
        let mut codes = Vec::new();

        for (&time, weather) in time_points.iter().zip(weather_points) {
            if !in_range(time) {
                continue;
            }
            temps.push(weather.temp);
            codes.push((weather.code, time.hour()));
            precips.push(format_precip(weather.precip));
        }

        table = table
            .group(model)
            .column(
                "",
                codes
                    .into_iter()
                    .map(|(c, h)| weather_symbol(c, h))
                    .collect(),
            )
            .column("Temp", temps.into_iter().map(format_temp).collect())
            .column("Precip", precips)
            .column("", vec![String::new(); num_rows]);
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
    verbose: bool,
) -> anyhow::Result<()> {
    let location = resolve_location(location).await?;
    let date_range = parse_date_range(dates)?;

    println!("Forecast for {}", location.display_name);

    let models: Vec<&str> = models.iter().map(|s| s.as_str()).collect();
    let mut forecast = Forecast::download(location.latitude, location.longitude, &models).await?;

    let now = Local::now()
        .with_timezone(&forecast.timezone)
        .fixed_offset();
    if !full {
        forecast.compress(now.date_naive());
    }

    let time_range = resolve_time_range(date_range, forecast.timezone, now);

    if verbose {
        println!("Grid-cell location: {}", forecast.location.link());
        println!("Timezone: {}", forecast.timezone);
        println!("Interval: [{}, {})", time_range.0, time_range.1);
    }

    build_forecast_table(&forecast.times, &forecast.by_model, time_range).print();
    Ok(())
}

/// Handle the `current` subcommand: fetch and display current weather.
///
/// Resolves the location (by name or coordinates), downloads current weather from Open-Meteo,
/// and prints the result as a single-row table.
async fn do_current(location: &str, verbose: bool) -> anyhow::Result<()> {
    let location = resolve_location(location).await?;

    println!("Current weather for {}", location.display_name);

    let current = Current::download(location.latitude, location.longitude).await?;

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
            vec![weather_symbol(current.weather.code, current.time.hour())],
        )
        .column("Temp", vec![format_temp(current.weather.temp)])
        .column("Precip", vec![format_precip(current.weather.precip)])
        .print();
    Ok(())
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
            verbose,
        } => do_forecast(&location, &dates, &models, full, verbose).await,
        Command::Current { location, verbose } => do_current(&location, verbose).await,
    }
}

mod time {
    use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveTime, Weekday};
    use chrono_tz::Tz;

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

    pub fn parse_date_range(s: &str) -> anyhow::Result<(RequestedDate, RequestedDate)> {
        if let Some(pos) = s.find("..") {
            let a = parse_date(&s[..pos])?;
            let b = parse_date(&s[pos + 2..])?;
            Ok((a, b))
        } else {
            let d = parse_date(s)?;
            Ok((d, d))
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
        (mut start_date, mut end_date): (RequestedDate, RequestedDate),
        timezone: Tz,
        relative_to: DateTime<FixedOffset>,
    ) -> (DateTime<FixedOffset>, DateTime<FixedOffset>) {
        use chrono::TimeZone;

        let original_date = relative_to.date_naive();

        // Open-Meteo provides forecasts at hour starts, so after 23:00 there's no more data
        // for "today". Since start is clamped to `relative_to`, shift to "tomorrow" to avoid
        // an empty forecast. We use 22:55 as the cutoff to account for network latency.
        const CUTOFF_TIME: NaiveTime = NaiveTime::from_hms_opt(22, 55, 0).unwrap();

        if relative_to.time() > CUTOFF_TIME {
            if start_date == RequestedDate::Today {
                start_date = RequestedDate::Tomorrow;
            }
            if end_date == RequestedDate::Today {
                end_date = RequestedDate::Tomorrow;
            }
        }

        // We've updated start and end date, but still pass the original relative_to to
        // resolve_date(), so that "+2" or "thursday" refer to the correct date.
        let start_resolved = resolve_date(start_date, original_date, original_date);
        let end_resolved = resolve_date(end_date, original_date, start_resolved);

        let start_time = timezone
            .from_local_datetime(&start_resolved.and_time(NaiveTime::MIN))
            .unwrap()
            .fixed_offset();
        let start_time = std::cmp::max(start_time, relative_to);

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
        fn parse_date_range_invalid() {
            assert!(parse_date_range("invalid..today").is_err());
            assert!(parse_date_range("today..invalid").is_err());
            assert!(parse_date_range("..today").is_err());
            assert!(parse_date_range("today..").is_err());
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
            let relative_to = make_time(23, 0); // after 22:55
            let (start, end) = test_resolve("today", relative_to);
            // "today" should shift to tomorrow due to cutoff
            assert_eq!(
                start.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 16).unwrap()
            );
            assert_eq!(start.hour(), 0);
            // End should be midnight of the day after tomorrow
            assert_eq!(
                end.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 17).unwrap()
            );
        }

        #[test]
        fn resolve_time_range_at_cutoff_boundary() {
            // Exactly at 22:55 should NOT trigger the shift (we use >)
            let (start, _) = test_resolve("today", make_time(22, 55));
            assert_eq!(
                start.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 15).unwrap()
            );

            // One minute later should trigger the shift
            let (start, _) = test_resolve("today", make_time(22, 56));
            assert_eq!(
                start.date_naive(),
                NaiveDate::from_ymd_opt(2025, 1, 16).unwrap()
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
