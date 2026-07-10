# WEATHER.md — living weather for Neisor

Status: Phase W1 landing (2026-07-09). A long-running thread; this doc is
the contract. Owner: Austin + Andrew (taste), Fable (architecture).

## What we're building

The planet already knows its weather: planetgen ships 12-month fields for
temperature, precipitation, wind, cloud fraction, and sea ice at every one
of 655k cells. The viewer currently uses only annual means. This thread
makes the sky and the seasons real: clouds with gradations, rain and snow
that fall, ground and trees that answer the season, frozen things that can
someday melt — all tunable, all reproducible.

## The three-layer architecture

**Layer 1 — climatology (data, slow):** what the planet's climate DOES
here at this time of year. Baked from the monthly fields into
`viewer/assets/weather.bin` as smooth harmonics per texel (mean + annual
harmonic, plus a semi-annual term for precip/clouds — monsoon regimes are
not sinusoidal). Query: `climate_at(dir, season) -> {temp_c, precip_norm,
cloud_norm, wind}`. Changing the planet's climate = rebaking data, never
touching code.

**Layer 2 — synoptic (procedural, live):** what the weather is doing THIS
HOUR: storm systems, clear spells, fronts. A stateless field — noise
octaves whose domain is advected along the climatological wind and whose
amplitude/bias comes from Layer 1 — evaluated as a pure function
`weather_at(dir, t_s) -> {cloud_cover, precip, snow_frac, temp_c, wind,
storminess}`. This IS the "real-time sim" and the answer to feasibility:
storms drift with the wind, grow and die over tens of minutes, and yet
there is no mutable state anywhere — any (seed, position, time) always
reproduces the identical weather. Seekable, rewindable, deterministic.

**Layer 3 — presentation (renderer, per-frame):** what you SEE: the cloud
shell in the sky, sun dimming under overcast, precipitation particles,
snow dusting and rain-darkening on the ground, and someday cloud shadows,
fog banks, lightning. Reads Layer 2 once per frame at the camera (cheap)
plus per-pixel procedural detail in the shaders.

## The determinism contract (non-negotiable)

- Weather is a pure function of (planet seed, position, weather time).
  No accumulation, no RNG draws at runtime, no wall clock.
- Weather time = the renderer's `render_time_s` (+ a configurable epoch).
  The play harness already drives render time from its fixed sim clock
  (F-20), so every `.play` script — including every EXISTING suite —
  reproduces byte-identical frames with weather on. Double-run hash gates
  keep proving it.
- Scripts get a `weather` command (pin time/intensity or off), mirroring
  `sun`. Shot sidecars record the weather state; `repro_shot.py` replays
  it. A photo of a storm is a coordinate you can teleport back into.
- Phase W1 touches NOTHING structural: terrain::sample still reads annual
  means, so every census number and physics assert is untouched by
  construction. Structural seasonality (W4) is where that changes, gated.

## The TRANSITIONS.md boundary (do not muddy)

Weather must never create a new mesh-vs-voxel disagreement:
- Ground responses (snow dusting, rain darkening) live in the SHARED
  fragment shader, applied identically to mesh tiles and voxel chunks by
  construction — never in per-renderer geometry builders.
- Anything that would change sample() (structural seasonality) is a
  shared-sample change reviewed against the transitions program first.
- Sky/particles are renderer-global and touch neither.

## Time model (all tunable)

- `day_len_s` (exists): 1200 s default.
- `days_per_year`: 20 default -> a year is ~6.7 h of play; a session sees
  the season move. Andrew call: longer for realism, shorter for spectacle.
- `weather_epoch_frac`: where in the year the world starts (default:
  northern early summer, 0.45).
- Synoptic pace: storm systems ~800 km across drifting at ~10x real wind
  speed so a front crosses your valley in ~15 minutes. Tunable multiplier.

## Gradations (Austin's ask, explicitly)

- Sky: clear -> scattered -> broken -> overcast -> storm, a continuous
  `cloud_cover` in [0,1] with a tunable contrast curve; never a binary.
- Precipitation: `precip` in [0,1] continuous (drizzle -> downpour),
  split rain/snow by local air temperature with a mixed band around 0 C
  (sleety flurries are allowed and good).
- Ground: dusting -> cover for snow; damp -> soaked darkening for rain.

## Hard constraints

- From orbit the ground is never fully obscured: the orbital cloud layer
  (W3) has a hard tunable opacity cap (default 0.55) and the cloud shell
  fades out above ~15 km camera altitude in W1 (i.e., W1 renders NO
  clouds from space — trivially satisfying the constraint until W3 does
  it properly).
- Performance: Layer 2 evaluated per-frame at the camera + a handful of
  probe points (< 20 samples); per-pixel work is shader noise only.

## Tuning surface

Every knob lives in `WeatherTuning` (viewer/src/weather.rs) with defaults
in code and an optional override file `viewer/assets/weather_tuning.json`
(same pattern as sidecars — serde_json, no new deps). Knobs include:
cloud shell altitude/scale/contrast/cap, storminess amplitude, synoptic
speed/scale, precip threshold/intensity curve, snow/rain split band,
dusting altitude falloff, darkening strength, particle counts/speeds,
year length, epoch. If Andrew wants to art-direct a value, it must be a
knob, not a constant.

## Phase roadmap

- **W1 (this commit):** weather.bin bake + weather.rs field + sky cloud
  shell (from below/aloft, fades before orbit) + sun/ambient dimming +
  precip particles (rain streaks / snow flakes) + snow dusting and rain
  darkening in the shared shader + `weather` play command + sidecar
  fields + weather-visual.play demo suite.
- **W2:** cloud shadows on the ground (project the shell noise along the
  sun), fog banks / valley mist by humidity + dawn, storm-edge lighting
  (dark horizons, shafts), wind-driven particle slant.
- **W3:** orbital clouds — a proper capped-opacity cloud layer visible
  from space, matched to the shell from below; the "planet from orbit
  breathes" shot. Hard cap enforced here.
- **W4 (gated on Andrew + suite strategy):** structural seasonality —
  frozen lakes melt in summer, sea ice advances/retreats (seaice_monthly
  is already baked per month!), snow line moves in sample(), deciduous
  tree color cycles. This changes physics (walkable ice comes and goes)
  and therefore the regression-suite time model: suites pin a canonical
  date; seasonal asserts get their own scripts. Design doc addendum
  before any code.
- **W5+ (dream list):** lightning + thunder delay, rainbows (sun-opposite
  arc when sunny+rain), aurora at high latitude night, dust storms in
  deserts (precip=0 + high wind), wind-swayed trees/shrubs, puddle
  accumulation on flats, snow depth as voxel overlay, weather-aware
  ambient audio.

## Decision points for Andrew (none block W1)

Moved to DECISIONS.md (repo root): D-5 year length/starting season,
D-6 cloud art direction, D-7 overcast gloom cap, D-8 storm frequency,
D-9 W4 seasonal ice/melt scope.

## Data format: weather.bin

```
magic  b"WEA1"
u32    res (texels per face edge, 256)
u32    n_layers (10)
6 faces x n_layers x res^2 f32, v-major rows, layer order:
  temp_a_c, temp_b_c        # temp mean lives in face_*.bin already
  prc_mean, prc_a, prc_b    # mm/month
  cld_mean, cld_a, cld_b    # cloud fraction 0..1
  wind_e, wind_n            # m/s annual mean
value(t_yr) = mean + a*cos(2*pi*t_yr) + b*sin(2*pi*t_yr), t_yr in [0,1)
with month centers at (m+0.5)/12. CARTESIAN coefficients on purpose:
amp/phase cannot be blended across texels (phase wraps — a texel between
a January-peak and a December-peak cell would average to July).
```
Rasters are edge-inclusive like face_*.bin (texel 0 and res-1 on the cube
edge) and sampled bilinearly; weather varies at synoptic scales, so 256
per face (~40 km/texel) is deliberately coarse and cheap (~16 MB).
