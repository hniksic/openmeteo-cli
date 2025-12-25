#!/usr/bin/env python3
"""
Historical data collector using Open-Meteo's Previous Runs and Historical Weather APIs.

Retrieves historical forecasts and observations to enable forecast accuracy analysis
without needing real-time collection.

Note: Data availability varies by model:
- Most models: January 2024 onward
- GFS: March 2021 onward
- JMA: 2018 onward

Usage:
    python3 collect_history.py [--start DATE] [--end DATE] [--max-past-days N]
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

import requests

from config import DATA_DIR, LOCATIONS, MODELS

# API endpoints
PREVIOUS_RUNS_API = "https://previous-runs-api.open-meteo.com/v1/forecast"
HISTORICAL_API = "https://archive-api.open-meteo.com/v1/archive"

# Rate limiting (be nice to the free API - Previous Runs API has stricter limits)
RATE_LIMIT_DELAY = 10.0

# Max models per request to avoid overloading the API
MODELS_PER_BATCH = 4


def log(msg: str) -> None:
    """Log message with timestamp to stderr."""
    print(f"[{datetime.now().isoformat()}] {msg}", file=sys.stderr)


def fetch_json(url: str, params: dict, max_retries: int = 5) -> dict | None:
    """Fetch JSON from API with error handling and retry for transient errors."""
    for attempt in range(max_retries):
        try:
            response = requests.get(url, params=params, timeout=120)
            response.raise_for_status()
            return response.json()
        except requests.exceptions.HTTPError as e:
            status = e.response.status_code if e.response else 0
            # Retry on: no response (0), rate limit (429), server errors (5xx)
            is_retryable = status == 0 or status == 429 or status >= 500
            if is_retryable and attempt < max_retries - 1:
                wait = (2 ** attempt) * 15  # 15s, 30s, 60s, 120s
                log(f"  ⚠ HTTP {status}, waiting {wait}s before retry ({attempt + 1}/{max_retries})...")
                time.sleep(wait)
                continue
            log(f"  ✗ HTTP {status} after {attempt + 1} attempts, giving up")
            return None
        except (requests.exceptions.Timeout, requests.exceptions.ConnectionError) as e:
            if attempt < max_retries - 1:
                wait = (2 ** attempt) * 15
                log(f"  ⚠ Connection error, waiting {wait}s before retry ({attempt + 1}/{max_retries})...")
                time.sleep(wait)
                continue
            log(f"  ✗ Connection error after {attempt + 1} attempts: {e}")
            return None
        except Exception as e:
            log(f"  ✗ Unexpected error: {e}")
            return None
    return None


def format_utc_offset(seconds: int) -> str:
    """Format UTC offset seconds as +HH:MM or -HH:MM."""
    sign = "+" if seconds >= 0 else "-"
    seconds = abs(seconds)
    hours = seconds // 3600
    minutes = (seconds % 3600) // 60
    return f"{sign}{hours:02d}:{minutes:02d}"


def fetch_historical_observations(
    lat: float, lon: float, start_date: str, end_date: str
) -> dict | None:
    """Fetch historical weather observations."""
    params = {
        "latitude": lat,
        "longitude": lon,
        "start_date": start_date,
        "end_date": end_date,
        "hourly": "temperature_2m,precipitation,weather_code",
        "timezone": "auto",
    }
    return fetch_json(HISTORICAL_API, params)


def fetch_previous_runs(
    lat: float, lon: float, start_date: str, end_date: str,
    models: list[str], max_past_days: int
) -> dict | None:
    """Fetch forecasts from previous model runs."""
    # Build variable list with previous_dayN suffixes
    base_vars = ["temperature_2m", "precipitation", "weather_code"]
    hourly_vars = []
    for var in base_vars:
        # Current run (day 0)
        hourly_vars.append(var)
        # Previous runs
        for day in range(1, max_past_days + 1):
            hourly_vars.append(f"{var}_previous_day{day}")

    params = {
        "latitude": lat,
        "longitude": lon,
        "start_date": start_date,
        "end_date": end_date,
        "hourly": ",".join(hourly_vars),
        "models": ",".join(models),
        "timezone": "auto",
    }
    return fetch_json(PREVIOUS_RUNS_API, params)


def save_observations(location: str, data: dict) -> int:
    """Save historical observations in collect.py format. Returns count saved."""
    if not data or "hourly" not in data:
        return 0

    hourly = data["hourly"]
    times = hourly.get("time", [])
    temps = hourly.get("temperature_2m", [])
    precips = hourly.get("precipitation", [])
    codes = hourly.get("weather_code", [])
    utc_offset = data.get("utc_offset_seconds", 0)
    offset_str = format_utc_offset(utc_offset)

    count = 0
    for i, time_str in enumerate(times):
        if i >= len(temps) or temps[i] is None:
            continue

        # Add timezone offset to time string
        time_with_tz = f"{time_str}:00{offset_str}"

        # Parse time and organize by date/hour (using UTC for file paths)
        dt = datetime.fromisoformat(time_with_tz).astimezone(timezone.utc)
        date_str = dt.strftime("%Y-%m-%d")
        hour_str = dt.strftime("%H")

        output_dir = DATA_DIR / location / "current" / date_str
        output_dir.mkdir(parents=True, exist_ok=True)
        output_file = output_dir / f"{hour_str}.jsonl"

        record = {
            "time": time_with_tz,
            "latitude": data.get("latitude"),
            "longitude": data.get("longitude"),
            "temperature": temps[i],
            "precipitation": precips[i] if i < len(precips) and precips[i] is not None else 0.0,
            "weather_code": codes[i] if i < len(codes) and codes[i] is not None else 0,
        }

        with open(output_file, "w") as f:
            f.write(json.dumps(record) + "\n")
        count += 1

    return count


def save_forecasts(
    location: str, data: dict, models: list[str], max_past_days: int
) -> tuple[int, set[str]]:
    """Save previous run forecasts in collect.py format.

    Returns (count_saved, models_with_data).
    """
    if not data or "hourly" not in data:
        return 0, set()

    hourly = data["hourly"]
    times = hourly.get("time", [])
    utc_offset = data.get("utc_offset_seconds", 0)
    offset_str = format_utc_offset(utc_offset)

    if not times:
        return 0, set()

    count = 0
    models_with_data: set[str] = set()

    # Process each model
    for model in models:
        # Model suffix in variable names (empty for single model, otherwise _modelname)
        model_suffix = f"_{model}" if len(models) > 1 else ""

        # Process each previous day's run (including day 0 = current)
        for past_day in range(max_past_days + 1):
            day_suffix = "" if past_day == 0 else f"_previous_day{past_day}"

            # Build variable names
            temp_var = f"temperature_2m{day_suffix}{model_suffix}"
            precip_var = f"precipitation{day_suffix}{model_suffix}"
            code_var = f"weather_code{day_suffix}{model_suffix}"

            temps = hourly.get(temp_var, [])
            precips = hourly.get(precip_var, [])
            codes = hourly.get(code_var, [])

            if not temps:
                continue

            models_with_data.add(model)

            # Determine when this forecast was made
            first_time = datetime.fromisoformat(f"{times[0]}:00{offset_str}")
            forecast_made_date = first_time.date() - timedelta(days=past_day)
            forecast_made = datetime(
                forecast_made_date.year,
                forecast_made_date.month,
                forecast_made_date.day,
                0,  # Assume 00 UTC run
                tzinfo=timezone.utc,
            )

            date_str = forecast_made.strftime("%Y-%m-%d")
            hour_str = forecast_made.strftime("%H")

            output_dir = DATA_DIR / location / "forecast" / date_str
            output_dir.mkdir(parents=True, exist_ok=True)
            output_file = output_dir / f"{hour_str}.jsonl"

            # Append to file (multiple models per file)
            records = []
            for i, time_str in enumerate(times):
                if i >= len(temps) or temps[i] is None:
                    continue

                time_with_tz = f"{time_str}:00{offset_str}"
                record = {
                    "model": model,
                    "time": time_with_tz,
                    "latitude": data.get("latitude"),
                    "longitude": data.get("longitude"),
                    "temperature": temps[i],
                    "precipitation": precips[i] if i < len(precips) and precips[i] is not None else 0.0,
                    "weather_code": codes[i] if i < len(codes) and codes[i] is not None else 0,
                }
                records.append(record)
                count += 1

            if records:
                with open(output_file, "a") as f:
                    for record in records:
                        f.write(json.dumps(record) + "\n")

    return count, models_with_data


def check_observations_exist(location: str, start_date: str, end_date: str) -> bool:
    """Check if observations already exist for the date range."""
    # Check the last day of chunk - avoids UTC offset issues with first day
    # (local midnight may fall in previous UTC day)
    day_dir = DATA_DIR / location / "current" / end_date
    return day_dir.exists() and any(day_dir.glob("*.jsonl"))


def check_forecasts_exist(
    location: str, start_date: str, max_past_days: int, models: list[str]
) -> tuple[bool, set[str]]:
    """Check if forecasts already exist for all expected models.

    Returns (is_complete, models_found).
    """
    forecast_dir = DATA_DIR / location / "forecast" / start_date
    if not forecast_dir.exists():
        return False, set()

    # Read the first file and check which models are present
    jsonl_files = list(forecast_dir.glob("*.jsonl"))
    if not jsonl_files:
        return False, set()

    models_found: set[str] = set()
    try:
        with open(jsonl_files[0]) as f:
            for line in f:
                line = line.strip()
                if line:
                    data = json.loads(line)
                    models_found.add(data.get("model", ""))
    except (json.JSONDecodeError, IOError):
        return False, set()

    # Check if all expected models are present
    expected = set(models)
    return expected <= models_found, models_found


def collect_historical_data(
    start_date: datetime, end_date: datetime, models: list[str], max_past_days: int
) -> None:
    """Collect historical observations and forecasts for all locations."""
    # Process in chunks to avoid API limits
    chunk_days = 14  # Smaller chunks for more data per request

    # Calculate total chunks for progress reporting
    total_days = (end_date - start_date).days
    total_chunks = (total_days + chunk_days - 1) // chunk_days
    chunk_num = 0
    start_time = time.time()

    current = start_date
    while current < end_date:
        chunk_num += 1
        chunk_end = min(current + timedelta(days=chunk_days), end_date)
        start_str = current.strftime("%Y-%m-%d")
        end_str = chunk_end.strftime("%Y-%m-%d")

        # Calculate progress and ETA
        days_done = (current - start_date).days
        pct = int(100 * days_done / total_days) if total_days > 0 else 0
        elapsed = time.time() - start_time
        if chunk_num > 1:
            avg_per_chunk = elapsed / (chunk_num - 1)
            remaining_chunks = total_chunks - chunk_num + 1
            eta_seconds = avg_per_chunk * remaining_chunks
            eta_str = f", ETA: {int(eta_seconds // 60)}m {int(eta_seconds % 60)}s"
        else:
            eta_str = ""

        log(f"[{pct}%] Processing {start_str} to {end_str}{eta_str}")

        for location, (lat, lon) in LOCATIONS.items():
            # Check and fetch observations
            if check_observations_exist(location, start_str, end_str):
                log(f"  {location}: observations exist, skipping")
            else:
                obs_data = fetch_historical_observations(lat, lon, start_str, end_str)
                if obs_data:
                    obs_count = save_observations(location, obs_data)
                    log(f"  {location}: {obs_count} observations")
                time.sleep(RATE_LIMIT_DELAY)

            # Check and fetch forecasts (in batches to avoid overloading API)
            forecasts_complete, models_in_file = check_forecasts_exist(
                location, start_str, max_past_days, models
            )
            if forecasts_complete:
                log(f"  {location}: forecasts exist, skipping")
            else:
                # Only fetch missing models when resuming
                if models_in_file:
                    models_to_fetch = sorted(set(models) - models_in_file)
                    log(f"  {location}: forecasts incomplete, missing {len(models_to_fetch)} models: "
                        f"{', '.join(models_to_fetch[:5])}{'...' if len(models_to_fetch) > 5 else ''}")
                else:
                    models_to_fetch = models

                total_fc_count = 0
                all_models_found: set[str] = set()

                # Split models into batches
                num_batches = (len(models_to_fetch) + MODELS_PER_BATCH - 1) // MODELS_PER_BATCH
                for batch_idx, batch_start in enumerate(range(0, len(models_to_fetch), MODELS_PER_BATCH)):
                    batch = models_to_fetch[batch_start:batch_start + MODELS_PER_BATCH]
                    log(f"  {location}: fetching batch {batch_idx + 1}/{num_batches} "
                        f"({len(batch)} models)...")
                    forecast_data = fetch_previous_runs(
                        lat, lon, start_str, end_str, batch, max_past_days
                    )
                    if forecast_data:
                        fc_count, models_found = save_forecasts(
                            location, forecast_data, batch, max_past_days
                        )
                        total_fc_count += fc_count
                        all_models_found.update(models_found)
                    time.sleep(RATE_LIMIT_DELAY)

                still_missing = set(models_to_fetch) - all_models_found
                if still_missing:
                    log(f"  {location}: {total_fc_count} forecasts ({len(all_models_found)}/{len(models_to_fetch)} "
                        f"models, still missing: {', '.join(sorted(still_missing)[:5])}...)")
                else:
                    log(f"  {location}: {total_fc_count} forecasts ({len(all_models_found)} models)")

        current = chunk_end + timedelta(days=1)


def main() -> None:
    # Default to 1 year ago
    one_year_ago = (datetime.now(timezone.utc) - timedelta(days=365)).strftime("%Y-%m-%d")

    parser = argparse.ArgumentParser(
        description="Collect historical weather data from Open-Meteo"
    )
    parser.add_argument(
        "--start", "-s",
        default=one_year_ago,
        help=f"Start date (YYYY-MM-DD), default: {one_year_ago}"
    )
    parser.add_argument(
        "--end", "-e",
        default=datetime.now(timezone.utc).strftime("%Y-%m-%d"),
        help="End date (YYYY-MM-DD), default: today"
    )
    parser.add_argument(
        "--max-past-days", "-p",
        type=int,
        default=14,
        help="Maximum past days for forecast runs (default: 14)"
    )
    parser.add_argument(
        "--models", "-m",
        default=",".join(MODELS),
        help=f"Comma-separated list of models (default: all {len(MODELS)} from config.py)"
    )
    args = parser.parse_args()

    start_date = datetime.strptime(args.start, "%Y-%m-%d")
    end_date = datetime.strptime(args.end, "%Y-%m-%d")
    models = [m.strip() for m in args.models.split(",")]

    log(f"Collecting historical data from {args.start} to {args.end}")
    log(f"Max past days for forecasts: {args.max_past_days}")
    log(f"Models: {', '.join(models)}")
    log(f"Locations: {', '.join(LOCATIONS.keys())}")

    overall_start = time.time()
    collect_historical_data(start_date, end_date, models, args.max_past_days)
    elapsed = time.time() - overall_start

    log(f"Collection complete! Total time: {int(elapsed // 60)}m {int(elapsed % 60)}s")


if __name__ == "__main__":
    main()
