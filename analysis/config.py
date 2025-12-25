"""Configuration for weather forecast accuracy analysis."""

from pathlib import Path

# Locations to monitor: name -> (latitude, longitude)
LOCATIONS = {
    "sibenik": (43.74, 15.9),
    "zagreb": (45.82, 15.98),
    "veprinac": (45.34, 14.28),
}

# Global models
GLOBAL_MODELS = [
    "best_match",
    "ecmwf_ifs025",
    "ecmwf_ifs",
    "gfs_seamless",
    "gfs_global",
    "gfs_graphcast025",
    "icon_seamless",
    "icon_global",
    "gem_seamless",
    "gem_global",
    "jma_seamless",
    "jma_gsm",
    "kma_seamless",
    "kma_gdps",
    "cma_grapes_global",
    "bom_access_global",
]

# European regional models
EUROPEAN_MODELS = [
    "icon_eu",
    "icon_d2",
    "meteofrance_seamless",
    "meteofrance_arpege_world",
    "meteofrance_arpege_europe",
    "meteofrance_arome_france",
    "meteofrance_arome_france_hd",
    "knmi_seamless",
    "knmi_harmonie_arome_europe",
    "knmi_harmonie_arome_netherlands",
    "dmi_seamless",
    "dmi_harmonie_arome_europe",
    "ukmo_seamless",
    "ukmo_global_deterministic_10km",
    "ukmo_uk_deterministic_2km",
    "metno_seamless",
    "metno_nordic",
    "meteoswiss_icon_seamless",
    "meteoswiss_icon_ch1",
    "meteoswiss_icon_ch2",
    "italia_meteo_arpae_icon_2i",
]

# All models combined
MODELS = GLOBAL_MODELS + EUROPEAN_MODELS

# Hours at which to collect forecasts (4 times per day)
FORECAST_HOURS = [0, 6, 12, 18]

# Paths
SCRIPT_DIR = Path(__file__).parent
DATA_DIR = SCRIPT_DIR / "data"
OPENMETEO_BIN = SCRIPT_DIR.parent / "target" / "release" / "openmeteo"
