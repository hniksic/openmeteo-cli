mod location;
mod openmeteo_fetch;
mod table;

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, TimeZone, Timelike, Weekday,
};
use chrono_tz::Tz;
use clap::{Parser, Subcommand};

use itertools::Itertools;
use location::resolve_location;
use openmeteo_fetch::{Current, Forecast, Weather};
use table::Table;
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
        #[arg(long, default_value = "gfs_graphcast025,ecmwf_ifs025")]
        models: String,

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

#[derive(Debug, Clone)]
enum ParsedDate {
    Today,
    Tomorrow,
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

fn parse_date(s: &str) -> anyhow::Result<ParsedDate> {
    use anyhow::Context;
    let s = s.to_lowercase();
    match s.as_str() {
        "today" => Ok(ParsedDate::Today),
        "tomorrow" => Ok(ParsedDate::Tomorrow),
        _ => {
            if let Some(weekday) = parse_weekday(&s) {
                Ok(ParsedDate::Weekday(weekday))
            } else {
                NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                    .map(ParsedDate::Absolute)
                    .context(
                        "dates must be YYYY-MM-DD, YYYY-MM-DD..YYYY-MM-DD, 'today' or 'tomorrow'",
                    )
            }
        }
    }
}

fn parse_date_range(s: &str) -> anyhow::Result<(ParsedDate, ParsedDate)> {
    if let Some(pos) = s.find("..") {
        let a = parse_date(&s[..pos])?;
        let b = parse_date(&s[pos + 2..])?;
        Ok((a, b))
    } else {
        let d = parse_date(s)?;
        Ok((d.clone(), d))
    }
}

fn resolve_date(dt: &ParsedDate, today: NaiveDate, weekday_start_at: NaiveDate) -> NaiveDate {
    match dt {
        ParsedDate::Today => today,
        ParsedDate::Tomorrow => today + Duration::days(1),
        ParsedDate::Weekday(wanted) => {
            let mut date = weekday_start_at;
            while date.weekday() != *wanted {
                date += Duration::days(1);
            }
            date
        }
        ParsedDate::Absolute(d) => *d,
    }
}

fn resolve_time_range(
    start_date: &ParsedDate,
    end_date: &ParsedDate,
    timezone: Tz,
) -> (DateTime<FixedOffset>, DateTime<FixedOffset>) {
    let now = Local::now().with_timezone(&timezone);
    let today = now.date_naive();

    // Adjust for late hour
    let (start_date, end_date) = if now.hour() == 23 {
        let start = match start_date {
            ParsedDate::Today => ParsedDate::Tomorrow,
            other => other.clone(),
        };
        let end = match end_date {
            ParsedDate::Today => ParsedDate::Tomorrow,
            other => other.clone(),
        };
        (start, end)
    } else {
        (start_date.clone(), end_date.clone())
    };

    let start_resolved = resolve_date(&start_date, today, today);
    let end_resolved = resolve_date(&end_date, today, start_resolved);

    let start_time = timezone
        .from_local_datetime(&start_resolved.and_hms_opt(0, 0, 0).unwrap())
        .unwrap();
    let start_time = std::cmp::max(start_time, now).fixed_offset();

    let end_resolved = end_resolved + Duration::days(1);
    let end_time = timezone
        .from_local_datetime(&end_resolved.and_hms_opt(0, 0, 0).unwrap())
        .unwrap()
        .fixed_offset();

    (start_time, end_time)
}

fn wmo_symbol(code: i32, is_night: bool) -> &'static str {
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

fn is_night(hour: u32) -> bool {
    hour < 6 || hour >= 20
}

fn weather_symbol(code: Option<i32>, hour: u32) -> String {
    match code {
        None => "-".to_string(),
        Some(c) => {
            let sym = wmo_symbol(c, is_night(hour));
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

fn build_forecast_table(
    times: &[DateTime<FixedOffset>],
    by_model: &[(String, Vec<Weather>)],
    (start_time, end_time): (DateTime<FixedOffset>, DateTime<FixedOffset>),
) -> Table {
    let in_range = |dt| dt >= start_time && dt < end_time;

    // Date column (deduped)
    let dates = dedup(
        times
            .iter()
            .filter(|&&dt| in_range(dt))
            .map(|dt| dt.format("%Y-%m-%d").to_string()),
    );

    // Hour column
    let hours: Vec<String> = times
        .iter()
        .filter(|&&dt| in_range(dt))
        .map(|dt| dt.format("%Hh").to_string())
        .collect();

    let num_rows = dates.len();

    let mut table = Table::new().column("Date", dates).column("Hour", hours);

    for (model, data) in by_model {
        let mut temps = Vec::new();
        let mut precips = Vec::new();
        let mut codes = Vec::new();
        let mut code_hours = Vec::new();

        for (&time, weather) in times.iter().zip(data) {
            if !in_range(time) {
                continue;
            }
            temps.push(weather.temp);
            codes.push(weather.code);
            code_hours.push(time.hour());
            precips.push(match weather.precip {
                Some(0.0) => String::new(),
                Some(p) => format!("{p}"),
                None => "-".to_string(),
            });
        }

        table = table
            .group(model)
            .column(
                "",
                codes
                    .iter()
                    .zip(&code_hours)
                    .map(|(c, h)| weather_symbol(*c, *h))
                    .collect(),
            )
            .column(
                "Temp",
                temps
                    .iter()
                    .map(|t| match t {
                        Some(temp) => format!("{}°", temp.round() as i32),
                        None => "-".to_string(),
                    })
                    .collect(),
            )
            .column("Precip", precips)
            .column("", vec![String::new(); num_rows]);
    }

    table
}

fn do_forecast(location: &str, dates: &str, models: &str, verbose: bool) -> anyhow::Result<()> {
    let location = resolve_location(location)?;
    let (start_date, end_date) = parse_date_range(dates)?;

    println!("Forecast for {}", location.display_name);

    let models: Vec<String> = models.split(',').map(|s| s.to_string()).collect();
    let forecast = Forecast::download(location.latitude, location.longitude, &models)?;

    let time_range = resolve_time_range(&start_date, &end_date, forecast.timezone);

    if verbose {
        println!("Grid-cell location: {}", forecast.location.link());
        println!("Timezone: {}", forecast.timezone);
        println!("Interval: [{}, {})", time_range.0, time_range.1);
    }

    build_forecast_table(&forecast.times, &forecast.by_model, time_range).print();
    Ok(())
}

fn do_current(location: &str, verbose: bool) -> anyhow::Result<()> {
    let location = resolve_location(location)?;

    println!("Current weather for {}", location.display_name);

    let current = Current::download(location.latitude, location.longitude)?;

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
        .column(
            "Temp",
            vec![match current.weather.temp {
                Some(t) => format!("{}°", t.round() as i32),
                None => "-".to_string(),
            }],
        )
        .column(
            "Precip",
            vec![match current.weather.precip {
                Some(0.0) => String::new(),
                Some(p) => format!("{p}"),
                None => "-".to_string(),
            }],
        )
        .print();
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Forecast {
            location,
            dates,
            models,
            verbose,
        } => do_forecast(&location, &dates, &models, verbose),
        Command::Current { location, verbose } => do_current(&location, verbose),
    }
}
