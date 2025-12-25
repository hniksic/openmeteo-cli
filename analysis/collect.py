#!/usr/bin/env python3
"""
Hourly data collector for weather forecast accuracy analysis.

Run from cron every hour:
    0 * * * * cd /path/to/analysis && python3 collect.py >> collect.log 2>&1

Collects:
- Current weather for all locations (every hour)
- 15-day forecasts from all models (at 00, 06, 12, 18)
"""
from __future__ import annotations

import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

from config import DATA_DIR, FORECAST_HOURS, LOCATIONS, MODELS, OPENMETEO_BIN


def log(msg: str) -> None:
    """Log message with timestamp to stderr."""
    print(f"[{datetime.now().isoformat()}] {msg}", file=sys.stderr)


def run_openmeteo(args: list[str]) -> str | None:
    """Run openmeteo command and return stdout, or None on error."""
    cmd = [str(OPENMETEO_BIN)] + args
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        if result.returncode != 0:
            log(f"Command failed: {' '.join(cmd)}")
            log(f"stderr: {result.stderr}")
            return None
        return result.stdout
    except subprocess.TimeoutExpired:
        log(f"Command timed out: {' '.join(cmd)}")
        return None
    except Exception as e:
        log(f"Command error: {' '.join(cmd)}: {e}")
        return None


def collect_current(location: str, lat: float, lon: float, now: datetime) -> None:
    """Collect current weather for a location."""
    date_str = now.strftime("%Y-%m-%d")
    hour_str = now.strftime("%H")

    output_dir = DATA_DIR / location / "current" / date_str
    output_dir.mkdir(parents=True, exist_ok=True)
    output_file = output_dir / f"{hour_str}.jsonl"

    coords = f"{lat},{lon}"
    output = run_openmeteo(["current", coords, "--json"])
    if output:
        with open(output_file, "w") as f:
            f.write(output)
        log(f"Collected current weather for {location}")


def collect_forecast(location: str, lat: float, lon: float, now: datetime) -> None:
    """Collect 15-day forecast for a location from all models."""
    date_str = now.strftime("%Y-%m-%d")
    hour_str = now.strftime("%H")

    output_dir = DATA_DIR / location / "forecast" / date_str
    output_dir.mkdir(parents=True, exist_ok=True)
    output_file = output_dir / f"{hour_str}.jsonl"

    coords = f"{lat},{lon}"
    models_arg = ",".join(MODELS)
    output = run_openmeteo(["forecast", coords, "today..+14", "--models", models_arg, "--json"])
    if output:
        with open(output_file, "w") as f:
            f.write(output)
        log(f"Collected forecast for {location} ({len(MODELS)} models)")


def main() -> None:
    now = datetime.now(timezone.utc)
    current_hour = now.hour

    log(f"Starting collection run (hour={current_hour:02d})")

    # Always collect current weather
    for location, (lat, lon) in LOCATIONS.items():
        collect_current(location, lat, lon, now)

    # Collect forecasts at specified hours
    if current_hour in FORECAST_HOURS:
        log("Forecast collection hour - collecting forecasts")
        for location, (lat, lon) in LOCATIONS.items():
            collect_forecast(location, lat, lon, now)

    log("Collection run complete")


if __name__ == "__main__":
    main()
