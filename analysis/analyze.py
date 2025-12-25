#!/usr/bin/env python3
"""
Analyze weather forecast accuracy by comparing predictions to actual observations.

Usage:
    python3 analyze.py [--location LOCATION] [--model MODEL] [--verbose] [--top N]

Outputs RMSE and composite score for each model, with overall and per-location rankings.
Composite score weights: precipitation (50%), temperature (35%), WMO code (15%).
"""
from __future__ import annotations

import argparse
import json
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone

import numpy as np
import pandas as pd

from config import DATA_DIR, LOCATIONS

# Lead time buckets in days
LEAD_TIME_BUCKETS = [
    (0, 2, "0-2d"),
    (3, 5, "3-5d"),
    (6, 9, "6-9d"),
    (10, 14, "10-14d"),
]

# Composite score weights (must sum to 1.0)
WEIGHT_PRECIP = 0.50
WEIGHT_TEMP = 0.35
WEIGHT_WMO = 0.15


@dataclass
class Observation:
    """Actual weather observation."""
    time: datetime
    temperature: float
    precipitation: float
    weather_code: int


@dataclass
class Prediction:
    """Weather prediction from a model."""
    model: str
    prediction_made: datetime
    forecast_for: datetime
    temperature: float
    precipitation: float
    weather_code: int


def parse_time(time_str: str) -> datetime:
    """Parse ISO timestamp, preserving timezone offset."""
    # Replace 'Z' with '+00:00' for Python < 3.11 compatibility
    if time_str.endswith("Z"):
        time_str = time_str[:-1] + "+00:00"
    return datetime.fromisoformat(time_str)


def load_observations(location: str) -> dict[datetime, Observation]:
    """Load all observations for a location, indexed by hour."""
    observations = {}
    current_dir = DATA_DIR / location / "current"
    if not current_dir.exists():
        return observations

    for date_dir in current_dir.iterdir():
        if not date_dir.is_dir():
            continue
        for hour_file in date_dir.glob("*.jsonl"):
            try:
                with open(hour_file) as f:
                    for line in f:
                        line = line.strip()
                        if not line:
                            continue
                        data = json.loads(line)
                        time = parse_time(data["time"])
                        time = time.replace(minute=0, second=0, microsecond=0)
                        obs = Observation(
                            time=time,
                            temperature=data["temperature"],
                            precipitation=data.get("precipitation", 0.0),
                            weather_code=data["weather_code"],
                        )
                        observations[time] = obs
            except (json.JSONDecodeError, KeyError) as e:
                print(f"Warning: Error reading {hour_file}: {e}")

    return observations


def load_predictions(location: str) -> list[Prediction]:
    """Load all predictions for a location."""
    predictions = []
    forecast_dir = DATA_DIR / location / "forecast"
    if not forecast_dir.exists():
        return predictions

    for date_dir in forecast_dir.iterdir():
        if not date_dir.is_dir():
            continue
        prediction_date = date_dir.name
        for hour_file in date_dir.glob("*.jsonl"):
            prediction_hour = int(hour_file.stem)
            prediction_made = datetime(
                int(prediction_date[:4]),
                int(prediction_date[5:7]),
                int(prediction_date[8:10]),
                prediction_hour,
                tzinfo=timezone.utc,
            )
            try:
                with open(hour_file) as f:
                    for line in f:
                        line = line.strip()
                        if not line:
                            continue
                        data = json.loads(line)
                        forecast_for = parse_time(data["time"])
                        forecast_for = forecast_for.replace(minute=0, second=0, microsecond=0)
                        pred = Prediction(
                            model=data["model"],
                            prediction_made=prediction_made,
                            forecast_for=forecast_for,
                            temperature=data["temperature"],
                            precipitation=data.get("precipitation", 0.0),
                            weather_code=data["weather_code"],
                        )
                        predictions.append(pred)
            except (json.JSONDecodeError, KeyError) as e:
                print(f"Warning: Error reading {hour_file}: {e}")

    return predictions


def get_lead_time_bucket(lead_hours: float) -> str | None:
    """Get the bucket label for a lead time in hours."""
    lead_days = lead_hours / 24
    for min_days, max_days, label in LEAD_TIME_BUCKETS:
        if min_days <= lead_days <= max_days:
            return label
    return None


def rmse(errors: list[float]) -> float:
    """Calculate root mean square error."""
    return np.sqrt(np.mean(np.square(errors)))


def collect_errors(
    location: str, filter_model: str | None = None, verbose: bool = False
) -> pd.DataFrame:
    """Collect prediction errors for a location."""
    observations = load_observations(location)
    predictions = load_predictions(location)

    if verbose:
        print(f"Loaded {len(observations)} observations and {len(predictions)} predictions "
              f"for {location}")

    # Collect squared errors by (model, bucket)
    errors: dict[tuple[str, str], dict] = defaultdict(
        lambda: {"temp": [], "precip": [], "wmo": []}
    )

    for pred in predictions:
        if filter_model and pred.model != filter_model:
            continue

        obs = observations.get(pred.forecast_for)
        if obs is None:
            continue

        lead_hours = (pred.forecast_for - pred.prediction_made).total_seconds() / 3600
        if lead_hours < 0:
            continue

        bucket = get_lead_time_bucket(lead_hours)
        if bucket is None:
            continue

        errors[(pred.model, bucket)]["temp"].append(pred.temperature - obs.temperature)
        errors[(pred.model, bucket)]["precip"].append(pred.precipitation - obs.precipitation)
        errors[(pred.model, bucket)]["wmo"].append(pred.weather_code - obs.weather_code)

    rows = []
    for (model, bucket), errs in errors.items():
        if not errs["temp"]:
            continue
        rows.append({
            "location": location,
            "model": model,
            "lead_time": bucket,
            "n": len(errs["temp"]),
            "temp_rmse": rmse(errs["temp"]),
            "precip_rmse": rmse(errs["precip"]),
            "wmo_rmse": rmse(errs["wmo"]),
        })

    return pd.DataFrame(rows)


def add_composite_score(df: pd.DataFrame) -> pd.DataFrame:
    """Add normalized composite score to dataframe.

    Normalizes each metric to 0-1 range (relative to min/max in the data),
    then combines with weights. Lower score = better.
    """
    if df.empty:
        return df

    df = df.copy()

    # Normalize each metric to 0-1 range
    for col in ["temp_rmse", "precip_rmse", "wmo_rmse"]:
        min_val, max_val = df[col].min(), df[col].max()
        if max_val > min_val:
            df[f"{col}_norm"] = (df[col] - min_val) / (max_val - min_val)
        else:
            df[f"{col}_norm"] = 0.0

    # Composite score (lower = better)
    df["score"] = (
        WEIGHT_PRECIP * df["precip_rmse_norm"]
        + WEIGHT_TEMP * df["temp_rmse_norm"]
        + WEIGHT_WMO * df["wmo_rmse_norm"]
    )

    return df


def print_ranking(df: pd.DataFrame, title: str, top_n: int | None = None) -> None:
    """Print a ranking table."""
    if df.empty:
        return

    df = df.sort_values("score")
    if top_n:
        df = df.head(top_n)

    print(f"\n{title}")
    print("-" * 72)
    print(f"{'#':<3} {'Model':<32} {'N':>5} {'Precip':>7} {'Temp':>6} {'WMO':>5} {'Score':>7}")
    print(f"{'':3} {'':32} {'':>5} {'RMSE':>7} {'RMSE':>6} {'RMSE':>5} {'':>7}")
    print("-" * 72)

    for rank, (_, row) in enumerate(df.iterrows(), 1):
        model = row.get("model", row.name) if "model" in row else row.name
        print(f"{rank:<3} {model:<32} {int(row['n']):>5} "
              f"{row['precip_rmse']:>7.2f} {row['temp_rmse']:>6.2f} "
              f"{row['wmo_rmse']:>5.1f} {row['score']:>7.3f}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Analyze weather forecast accuracy")
    parser.add_argument("--location", "-l", help="Filter to specific location")
    parser.add_argument("--model", "-m", help="Filter to specific model")
    parser.add_argument("--verbose", "-v", action="store_true", help="Show detailed output")
    parser.add_argument("--top", "-t", type=int, default=10, help="Show top N models (default 10)")
    parser.add_argument("--all", "-a", action="store_true", help="Show all models (not just top N)")
    args = parser.parse_args()

    top_n = None if args.all else args.top

    # Collect data from all locations
    all_results = []
    locations = [args.location] if args.location else list(LOCATIONS.keys())

    for location in locations:
        if location not in LOCATIONS:
            print(f"Unknown location: {location}")
            continue
        df = collect_errors(location, filter_model=args.model, verbose=args.verbose)
        all_results.append(df)

    if not all_results or all(df.empty for df in all_results):
        print("No data found. Run collect.py first to gather data.")
        return

    results = pd.concat(all_results, ignore_index=True)

    if results.empty:
        print("No matching forecast-observation pairs found.")
        print("This is expected if you just started collecting data.")
        print("Wait for forecasts to 'mature' so observations exist for predicted times.")
        return

    bucket_order = [b[2] for b in LEAD_TIME_BUCKETS]
    results["lead_time"] = pd.Categorical(
        results["lead_time"], categories=bucket_order, ordered=True
    )

    print("\n" + "=" * 72)
    print("WEATHER FORECAST ACCURACY ANALYSIS")
    print("=" * 72)
    print(f"Locations: {', '.join(locations)}")
    print(f"Score weights: precip {WEIGHT_PRECIP:.0%}, temp {WEIGHT_TEMP:.0%}, "
          f"WMO {WEIGHT_WMO:.0%}")
    print(f"Total comparisons: {results['n'].sum():,}")

    # === OVERALL RANKING ===
    overall = results.groupby("model").agg({
        "n": "sum",
        "temp_rmse": "mean",
        "precip_rmse": "mean",
        "wmo_rmse": "mean",
    })
    overall = add_composite_score(overall)
    print_ranking(overall, "OVERALL RANKING (all locations, all lead times)", top_n)

    # === RANKING BY LEAD TIME ===
    print("\n" + "=" * 72)
    print("RANKING BY LEAD TIME")

    for bucket in bucket_order:
        bucket_data = results[results["lead_time"] == bucket]
        if bucket_data.empty:
            continue

        by_model = bucket_data.groupby("model").agg({
            "n": "sum",
            "temp_rmse": "mean",
            "precip_rmse": "mean",
            "wmo_rmse": "mean",
        })
        by_model = add_composite_score(by_model)
        print_ranking(by_model, f"Lead time: {bucket}", top_n)

    # === RANKING BY LOCATION ===
    if len(locations) > 1:
        print("\n" + "=" * 72)
        print("BEST MODELS BY LOCATION")

        for location in locations:
            loc_data = results[results["location"] == location]
            if loc_data.empty:
                continue

            by_model = loc_data.groupby("model").agg({
                "n": "sum",
                "temp_rmse": "mean",
                "precip_rmse": "mean",
                "wmo_rmse": "mean",
            })
            by_model = add_composite_score(by_model)
            print_ranking(by_model, f"Location: {location.upper()}", top_n)

    # === SUMMARY: BEST MODEL PER CATEGORY ===
    print("\n" + "=" * 72)
    print("SUMMARY: BEST MODELS")
    print("-" * 72)

    # Overall best
    best_overall = overall.sort_values("score").index[0]
    print(f"{'Overall best:':<25} {best_overall}")

    # Best per lead time
    for bucket in bucket_order:
        bucket_data = results[results["lead_time"] == bucket]
        if bucket_data.empty:
            continue
        by_model = bucket_data.groupby("model").agg({
            "n": "sum", "temp_rmse": "mean", "precip_rmse": "mean", "wmo_rmse": "mean"
        })
        by_model = add_composite_score(by_model)
        best = by_model.sort_values("score").index[0]
        print(f"{'Best for ' + bucket + ':':<25} {best}")

    # Best per location
    if len(locations) > 1:
        for location in locations:
            loc_data = results[results["location"] == location]
            if loc_data.empty:
                continue
            by_model = loc_data.groupby("model").agg({
                "n": "sum", "temp_rmse": "mean", "precip_rmse": "mean", "wmo_rmse": "mean"
            })
            by_model = add_composite_score(by_model)
            best = by_model.sort_values("score").index[0]
            print(f"{'Best for ' + location + ':':<25} {best}")

    print()


if __name__ == "__main__":
    main()
