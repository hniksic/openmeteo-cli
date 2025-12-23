## Simple Open-Meteo retrieval CLI

This program retrieves temperature and precipitation forecast and current data from
Open-Meteo. It is designed to make it simple to retrieve and compare forecasts by
different models exposed by open-meteo, in particular the [DeepMind
GraphCast](https://deepmind.google/discover/blog/graphcast-ai-model-for-faster-and-more-accurate-global-weather-forecasting/)
AI-based model, which Google claims to be more accurate than ECMWF's state-of-the-art HRES
model. GraphCast forecasts have been [integrated into
Open-Meteo](https://openmeteo.substack.com/p/exploring-graphcast) since April 2024.

## Forecast

The `forecast` subcommand shows the forecast for a location and a range of dates:

```
openmeteo forecast zagreb tomorrow
```

Location is typically a place name such as `London`, `New York`, or more detailed `London,
Ontario`. Names are resolved using OpenStreetmap's [Nominatim search
API](https://nominatim.org/release-docs/develop/api/Search/). You can also directly
specify geographic coordinate such as pair of latitude and longitude, such as
`45.8150,15.9819`. In that case the Nominatim lookup will be omitted and the coordinate
used directly.

The date range is either a single date or a pair of beginning and end dates separated by
`..`. Date can be formatted as `YYYY-MM-DD`, or you can use shorthands `today`,
`tomorrow`, or a day of the week. Range is specified as `START..END`, such as
`2025-04-27..2025-05-01` or `mon..thu`. Note that ranges are inclusive, so the range
`mon..thu` includes Thursday. If the date range argument is omitted altogether, today's
forecast is shown.

The program outputs forecast as a text table. The first column is the date (with
consecutive equal dates omitted for readability), followed by hour, and then by
weather symbol, temperature and precipitation for each forecast model:

```
$ openmeteo forecast zagreb today
Forecast for Grad Zagreb, Hrvatska
                gfs_graphcast025 ecmwf_ifs025
Date       Hour    Temp Precip      Temp Precip
2025-12-23  12h 🌤   5°          🌧   5°    0.1
            13h ⛅   5°          🌧   5°    0.1
            14h ⛅   5°          🌧   5°    0.3
            ...
            22h ⛅   4°          🌧   4°    0.1
            23h ☁    4°          ☁    4°
```

## Current weather

The `current` subcommand shows the current weather for a location:

```
$ openmeteo current zagreb
Current weather for Grad Zagreb, Hrvatska
Time                Temp Precip
2025-12-21 23:15 ☁    4°
```

The location is interpreted the same as for `forecast`.

## Forecast models

You can use the `--models` argument to `forecast` to specify a comma-separated list of
forecast models to retrieve.  Each model will add two columns, one for temperature and one
for precipitation.  The default is to retrieve and show GraphCast (`gfs_graphcast025`) and
classic ECMWF (`ecmwf_ifs025`) forecasts.

Here is the current list of known models, taken from [the
documentation](https://open-meteo.com/en/docs). Note that not all of those work on all
locations, and many of them don't support full 16 days.

**Global models:**
* `best_match` - automatically selects best model for location
* `ecmwf_ifs025` / `ecmwf_ifs` - ECMWF IFS
* `gfs_seamless` / `gfs_global` - NCEP GFS
* `gfs_graphcast025` - GFS GraphCast
* `icon_seamless` / `icon_global` - DWD ICON
* `gem_seamless` / `gem_global` - GEM Global
* `jma_seamless` / `jma_gsm` - JMA GSM
* `kma_seamless` / `kma_gdps` - KMA GDPS
* `cma_grapes_global` - CMA GRAPES Global
* `bom_access_global` - BOM ACCESS Global

**European regional models:**
* `icon_eu` / `icon_d2` - DWD ICON EU / D2
* `meteofrance_seamless` / `meteofrance_arpege_world` / `meteofrance_arpege_europe` / `meteofrance_arome_france` / `meteofrance_arome_france_hd` - Météo-France
* `knmi_seamless` / `knmi_harmonie_arome_europe` / `knmi_harmonie_arome_netherlands` - KNMI Harmonie Arome
* `dmi_seamless` / `dmi_harmonie_arome_europe` - DMI Harmonie Arome
* `ukmo_seamless` / `ukmo_global_deterministic_10km` / `ukmo_uk_deterministic_2km` - UK Met Office
* `metno_seamless` / `metno_nordic` - MET Norway Nordic
* `meteoswiss_icon_seamless` / `meteoswiss_icon_ch1` / `meteoswiss_icon_ch2` - MeteoSwiss ICON
* `italia_meteo_arpae_icon_2i` - ItaliaMeteo ARPAE ICON 2I

**Other regional models:**
* `gem_regional` / `gem_hrdps_continental` / `gem_hrdps_west` - GEM HRDPS
* `jma_msm` - JMA MSM
* `kma_ldps` - KMA LDPS
* `gfs_hrrr` - NCEP HRRR U.S. Conus
* `ncep_nbm_conus` - NCEP NBM U.S. Conus
* `ncep_nam_conus` - NCEP NAM U.S. Conus
