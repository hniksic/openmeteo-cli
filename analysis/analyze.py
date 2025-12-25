#!/usr/bin/env python3
"""
Analyze weather forecast accuracy by comparing predictions to actual observations.

Usage:
    python3 analyze.py [--location LOCATION] [--model MODEL] [--verbose] [--top N]

Outputs RMSE and composite score for each model, with overall and per-location rankings.
Composite score weights: rain miss (40%), temperature (30%), precipitation (15%), WMO code (15%).
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

# Models to exclude from analysis (e.g., known to have bad data)
EXCLUDED_MODELS: set[str] = set()  # Can add model names here to exclude entirely

# Minimum error to consider a forecast "genuine" (filters out fake data where
# forecast exactly matches observation, which happens with some API issues)
MIN_GENUINE_ERROR = 0.01  # °C - forecasts matching obs closer than this are excluded

# Rain detection threshold (mm/hour)
RAIN_THRESHOLD = 0.1

# Composite score weights (must sum to 1.0)
# rain_miss = binary rain/no-rain misprediction (most important for outdoor planning)
# precip = precipitation amount error
# temp = temperature error
# wmo = weather code error
WEIGHT_RAIN_MISS = 0.40
WEIGHT_PRECIP = 0.15
WEIGHT_TEMP = 0.30
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


def collect_raw_errors(
    location: str, filter_model: str | None = None, verbose: bool = False
) -> pd.DataFrame:
    """Collect individual prediction errors for a location.

    Returns a DataFrame with one row per (model, observation_time, lead_bucket) tuple,
    containing the raw errors for proper RMSE pooling.
    """
    observations = load_observations(location)
    predictions = load_predictions(location)

    if verbose:
        print(f"Loaded {len(observations)} observations and {len(predictions)} predictions "
              f"for {location}")

    rows = []
    for pred in predictions:
        if pred.model in EXCLUDED_MODELS:
            continue
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

        # Calculate errors
        temp_err = pred.temperature - obs.temperature
        precip_err = pred.precipitation - obs.precipitation
        wmo_err = pred.weather_code - obs.weather_code

        # Filter out fake data (forecast exactly matches observation)
        if abs(temp_err) < MIN_GENUINE_ERROR and abs(precip_err) < MIN_GENUINE_ERROR:
            continue

        # Rain misprediction: did we get rain/no-rain wrong?
        pred_rain = pred.precipitation > RAIN_THRESHOLD
        obs_rain = obs.precipitation > RAIN_THRESHOLD
        rain_miss = 1 if pred_rain != obs_rain else 0

        rows.append({
            "location": location,
            "model": pred.model,
            "obs_time": pred.forecast_for,
            "lead_time": bucket,
            "temp_err": temp_err,
            "precip_err": precip_err,
            "wmo_err": wmo_err,
            "rain_miss": rain_miss,
        })

    return pd.DataFrame(rows)


def filter_common_observations(
    df: pd.DataFrame, min_model_fraction: float = 0.8, verbose: bool = False
) -> pd.DataFrame:
    """Filter to observation points where most models have data.

    Two-stage filter for fair comparison:
    1. Keep observations where ≥min_model_fraction of models have forecasts
    2. Keep only models that have data for ≥min_model_fraction of those observations

    This ensures all compared models are evaluated on the same observation set.
    """
    if df.empty:
        return df

    all_models = df["model"].nunique()
    min_models = max(1, int(all_models * min_model_fraction))

    # Stage 1: Keep observations with enough models
    obs_key = ["location", "obs_time", "lead_time"]
    models_per_obs = df.groupby(obs_key)["model"].nunique().reset_index()
    models_per_obs.columns = obs_key + ["model_count"]
    valid_obs = models_per_obs[models_per_obs["model_count"] >= min_models]
    stage1 = df.merge(valid_obs[obs_key], on=obs_key)

    if stage1.empty:
        return stage1

    # Stage 2: Keep only models with enough observations
    total_obs = valid_obs.shape[0]
    min_obs = max(1, int(total_obs * min_model_fraction))

    obs_per_model = stage1.groupby("model")[obs_key[0]].count().reset_index()
    obs_per_model.columns = ["model", "obs_count"]
    valid_models = obs_per_model[obs_per_model["obs_count"] >= min_obs]["model"]

    if verbose:
        excluded = set(stage1["model"].unique()) - set(valid_models)
        if excluded:
            print(f"Excluding models with insufficient data: {', '.join(sorted(excluded))}")

    stage2 = stage1[stage1["model"].isin(valid_models)]

    # Stage 3: Now that we have fewer models, re-filter observations
    # to only those where ALL remaining models have data
    remaining_models = stage2["model"].nunique()
    models_per_obs2 = stage2.groupby(obs_key)["model"].nunique().reset_index()
    models_per_obs2.columns = obs_key + ["model_count"]
    # Keep only observations where ALL remaining models have data
    valid_obs2 = models_per_obs2[models_per_obs2["model_count"] == remaining_models]
    final = stage2.merge(valid_obs2[obs_key], on=obs_key)

    return final


def compute_rmse_stats(df: pd.DataFrame) -> pd.DataFrame:
    """Compute RMSE statistics from raw errors using proper pooling."""
    if df.empty:
        return pd.DataFrame()

    def pooled_rmse(errors):
        return np.sqrt(np.mean(np.square(errors)))

    stats = df.groupby("model").agg(
        n=("temp_err", "count"),
        temp_rmse=("temp_err", pooled_rmse),
        precip_rmse=("precip_err", pooled_rmse),
        wmo_rmse=("wmo_err", pooled_rmse),
        rain_miss_rate=("rain_miss", "mean"),  # fraction of rain/no-rain mispredictions
    ).reset_index()

    return stats


def add_composite_score(df: pd.DataFrame) -> pd.DataFrame:
    """Add normalized composite score to dataframe.

    Normalizes each metric to 0-1 range (relative to min/max in the data),
    then combines with weights. Lower score = better.
    """
    if df.empty:
        return df

    df = df.copy()

    # Normalize each metric to 0-1 range
    for col in ["temp_rmse", "precip_rmse", "wmo_rmse", "rain_miss_rate"]:
        min_val, max_val = df[col].min(), df[col].max()
        if max_val > min_val:
            df[f"{col}_norm"] = (df[col] - min_val) / (max_val - min_val)
        else:
            df[f"{col}_norm"] = 0.0

    # Composite score (lower = better)
    df["score"] = (
        WEIGHT_RAIN_MISS * df["rain_miss_rate_norm"]
        + WEIGHT_PRECIP * df["precip_rmse_norm"]
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
    print("-" * 80)
    print(f"{'#':<3} {'Model':<28} {'N':>5} {'Rain':>6} {'Precip':>7} {'Temp':>6} {'WMO':>5} {'Score':>7}")
    print(f"{'':3} {'':28} {'':>5} {'Miss%':>6} {'RMSE':>7} {'RMSE':>6} {'RMSE':>5} {'':>7}")
    print("-" * 80)

    for rank, (_, row) in enumerate(df.iterrows(), 1):
        model = row.get("model", row.name) if "model" in row else row.name
        rain_pct = row['rain_miss_rate'] * 100
        print(f"{rank:<3} {model:<28} {int(row['n']):>5} "
              f"{rain_pct:>5.1f}% {row['precip_rmse']:>7.2f} {row['temp_rmse']:>6.2f} "
              f"{row['wmo_rmse']:>5.1f} {row['score']:>7.3f}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Analyze weather forecast accuracy")
    parser.add_argument("--location", "-l", help="Filter to specific location")
    parser.add_argument("--model", "-m", help="Filter to specific model")
    parser.add_argument("--verbose", "-v", action="store_true", help="Show detailed output")
    parser.add_argument("--top", "-t", type=int, default=10, help="Show top N models (default 10)")
    parser.add_argument("--all", "-a", action="store_true", help="Show all models (not just top N)")
    parser.add_argument(
        "--min-models", type=float, default=0.8,
        help="Min fraction of models required per observation (default 0.8)"
    )
    args = parser.parse_args()

    top_n = None if args.all else args.top

    # Collect raw errors from all locations
    all_errors = []
    locations = [args.location] if args.location else list(LOCATIONS.keys())

    for location in locations:
        if location not in LOCATIONS:
            print(f"Unknown location: {location}")
            continue
        df = collect_raw_errors(location, filter_model=args.model, verbose=args.verbose)
        all_errors.append(df)

    if not all_errors or all(df.empty for df in all_errors):
        print("No data found. Run collect.py first to gather data.")
        return

    raw_errors = pd.concat(all_errors, ignore_index=True)

    if raw_errors.empty:
        print("No matching forecast-observation pairs found.")
        print("This is expected if you just started collecting data.")
        print("Wait for forecasts to 'mature' so observations exist for predicted times.")
        return

    # Filter to common observation points for fair comparison
    total_before = len(raw_errors)
    models_before = raw_errors["model"].nunique()
    filtered_errors = filter_common_observations(raw_errors, args.min_models, verbose=args.verbose)
    total_after = len(filtered_errors)
    models_after = filtered_errors["model"].nunique()

    if filtered_errors.empty:
        print("No common observation points found where enough models have data.")
        print("Try lowering --min-models threshold (e.g., --min-models 0.5)")
        return

    bucket_order = [b[2] for b in LEAD_TIME_BUCKETS]
    filtered_errors["lead_time"] = pd.Categorical(
        filtered_errors["lead_time"], categories=bucket_order, ordered=True
    )

    print("\n" + "=" * 80)
    print("WEATHER FORECAST ACCURACY ANALYSIS")
    print("=" * 80)
    print(f"Locations: {', '.join(locations)}")
    print(f"Score weights: rain_miss {WEIGHT_RAIN_MISS:.0%}, precip {WEIGHT_PRECIP:.0%}, "
          f"temp {WEIGHT_TEMP:.0%}, WMO {WEIGHT_WMO:.0%}")
    print(f"Rain threshold: >{RAIN_THRESHOLD}mm/hour")
    print(f"Models: {models_after} (of {models_before} with data)")
    print(f"Comparisons: {total_after:,} (filtered from {total_before:,} for fair comparison)")
    print(f"Filter: observations where ≥{args.min_models:.0%} of models have forecasts")

    # === OVERALL RANKING ===
    overall = compute_rmse_stats(filtered_errors)
    overall = add_composite_score(overall)
    overall = overall.set_index("model")
    print_ranking(overall, "OVERALL RANKING (all locations, all lead times)", top_n)

    # === RANKING BY LEAD TIME ===
    print("\n" + "=" * 80)
    print("RANKING BY LEAD TIME")

    for bucket in bucket_order:
        bucket_data = filtered_errors[filtered_errors["lead_time"] == bucket]
        if bucket_data.empty:
            continue

        by_model = compute_rmse_stats(bucket_data)
        by_model = add_composite_score(by_model)
        by_model = by_model.set_index("model")
        print_ranking(by_model, f"Lead time: {bucket}", top_n)

    # === RANKING BY LOCATION ===
    if len(locations) > 1:
        print("\n" + "=" * 80)
        print("BEST MODELS BY LOCATION")

        for location in locations:
            loc_data = filtered_errors[filtered_errors["location"] == location]
            if loc_data.empty:
                continue

            by_model = compute_rmse_stats(loc_data)
            by_model = add_composite_score(by_model)
            by_model = by_model.set_index("model")
            print_ranking(by_model, f"Location: {location.upper()}", top_n)

    # === SUMMARY: BEST MODEL PER CATEGORY ===
    print("\n" + "=" * 80)
    print("SUMMARY: BEST MODELS")
    print("-" * 72)

    # Overall best
    best_overall = overall.sort_values("score").index[0]
    print(f"{'Overall best:':<25} {best_overall}")

    # Best per lead time
    for bucket in bucket_order:
        bucket_data = filtered_errors[filtered_errors["lead_time"] == bucket]
        if bucket_data.empty:
            continue
        by_model = compute_rmse_stats(bucket_data)
        by_model = add_composite_score(by_model)
        by_model = by_model.set_index("model")
        best = by_model.sort_values("score").index[0]
        print(f"{'Best for ' + bucket + ':':<25} {best}")

    # Best per location
    if len(locations) > 1:
        for location in locations:
            loc_data = filtered_errors[filtered_errors["location"] == location]
            if loc_data.empty:
                continue
            by_model = compute_rmse_stats(loc_data)
            by_model = add_composite_score(by_model)
            by_model = by_model.set_index("model")
            best = by_model.sort_values("score").index[0]
            print(f"{'Best for ' + location + ':':<25} {best}")

    print()


if __name__ == "__main__":
    main()
