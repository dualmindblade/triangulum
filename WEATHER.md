# WEATHER.md — living weather for Neisor

Status: W1 + clouds v2/W3 + W2.5 spatial deck/wind map + W4 structural seasons landed (2026-07-13). A long-running thread; this doc is
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

**Layer 3 — presentation (renderer, per-frame):** what you SEE: three
parallax cloud shells and their capped orbital composite, sun dimming under
overcast, precipitation particles,
snow dusting and rain-darkening on the ground, and someday cloud shadows,
fog banks, lightning. Camera-local ground/particle/fog responses read Layer 2
at the camera. Cloud SHAPE reads a deterministic 6x64x64 RGBA8 synoptic raster
at each shell-hit direction, then adds per-pixel formation fabric below the
raster scale. The teleport map's Clouds-now layer samples those exact bytes.

## The determinism contract (non-negotiable)

- Weather is a pure function of (planet seed, position, weather time).
  No accumulation, no RNG draws at runtime, no wall clock.
- Weather time = the renderer's deterministic `render_time_s` plus a seek
  offset (zero normally; `weather time T_S` / `--weather-time T_S` sets it).
  The offset changes weather only: the sun/day cycle stays on the base clock.
  The play harness already drives render time from its fixed sim clock
  (F-20), so every `.play` script — including every EXISTING suite —
  reproduces byte-identical frames with weather on. Double-run hash gates
  keep proving it.
- Scripts get `weather off|live|pin COVER PRECIP|time T_S|season FRAC`,
  mirroring `sun`. Shot sidecars record the weather state and absolute time;
  `repro_shot.py`, `--capture`, and the photo map's opt-in time restore replay
  it. Pre-weather sidecars are the deliberate cut line: they warn and fall
  back to live weather at time zero because their storm state is unknowable.
  A photo of a storm is a coordinate you can teleport back into.
- W2.5's synoptic raster is rebuilt from scratch at a deterministic two-second
  absolute-weather-time bucket. It is never advected incrementally and never
  keyed by frame count. Live replay/cold capture therefore upload identical
  98,304 bytes; pins upload a uniform cover/precip raster (the shader retains
  exact scalar pin values to avoid UNORM8 rounding changes in old reels).
- Weather presentation touches NOTHING structural: terrain::sample still reads annual
  means, so every census number and physics assert is untouched by
  construction. Structural seasonality (W4) is where that changes, gated.

## The TRANSITIONS.md boundary (do not muddy)

Weather must never create a new mesh-vs-voxel disagreement:
- Ground responses (snow dusting, rain darkening) live in the SHARED
  fragment shader, applied identically to mesh tiles and voxel chunks by
  construction — never in per-renderer geometry builders.
- D-8's rain interpolation is one shared signed vertex channel. Mesh and
  voxel builders encode the same local-height residual into a previously
  spare byte; the fragment shader alone turns it into rain intensity.
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
- Formation type is continuous too: low cover favors high fibrous cirrus;
  middle cover grows broken cumulus masses; precipitation brings in a low,
  dark base aligned with the middle mass so warm storms read taller. Cold
  air strengthens ice-rich cirrus while warmth slightly favors vertical
  cumulus/storm growth.
- Precipitation: `precip` in [0,1] continuous (drizzle -> downpour),
  split rain/snow by local air temperature with a mixed band around 0 C
  (sleety flurries are allowed and good).
- Ground: dusting -> cover for snow; damp -> soaked darkening for rain.
  Rain intensity is subtly redistributed toward local troughs (at most 18%
  with defaults), not converted into a new biome or a new weather event.

## Hard constraints

- From orbit the ground is never fully obscured: low/middle/high shells are
  composited far-to-near and their COMBINED alpha is then hard-capped by
  `orbit_cloud_opacity_cap` (default 0.55). The below-deck path hands over
  between the 8.2 km high shell and 15 km camera altitude.
- Performance: camera responses and W2 compass probes retain their small
  per-frame sample budget. W2.5 additionally evaluates 24,576 independent
  raster texels once per deterministic two-second bucket (parallel CPU bake,
  ~4-7 ms on the gate machine, ~0.03-0.06 ms/frame amortized at 60 fps).
  Each visible/shadow cloud shell sample adds one filtered texture read.

## Tuning surface

Every knob lives in `WeatherTuning` (viewer/src/weather.rs) with defaults
in code and an optional override file `viewer/assets/weather_tuning.json`
(same pattern as sidecars — serde_json, no new deps). Knobs include:
cloud shell altitude/scale/contrast/cap, storminess amplitude, synoptic
speed/scale, precip threshold/intensity curve, snow/rain split band,
dusting altitude falloff, darkening strength, particle counts/speeds,
year length, epoch. If Andrew wants to art-direct a value, it must be a
knob, not a constant.

Clouds-v2 and D-8 defaults (the documented art knobs):

| knob | default | meaning |
|---|---:|---|
| `cloud_layer_count` | 3 | 1 = middle only, 2 = + high cirrus, 3 = + low storm base |
| `shell_alt_km` / `cloud_mid_alt_km` / `cloud_high_alt_km` | 1.8 / 3.8 / 8.2 | concentric shell altitudes |
| `shell_fade_km` | 15.0 | below-deck to orbital handoff end |
| `cloud_low_scale` / `cloud_mid_scale` / `cloud_high_scale` | 460 / 900 / 620 | unit-sphere noise frequencies; larger means smaller formations |
| `cloud_low_density` / `cloud_mid_density` / `cloud_high_density` | 0.92 / 0.82 / 0.32 | per-shell opacity multipliers before stacking |
| `orbit_cloud_opacity_cap` | 0.55 | hard cap after all orbital layers are stacked |
| `rain_crevice_bias` | 0.18 | maximum signed rain redistribution at deep trough/peak proxies |
| `wind_map_density` | 180 | target comet streamlines in the visible Neisor map window |
| `wind_map_length_km` | 1250 | whole-map streamline length; scales down with zoom |

The crevice proxy is `(coarse_elevation - detailed_elevation) / 120 m`,
clamped to `[-1,1]`. The coarse term is the smooth ~30 km elevation raster;
the detailed term includes band-limited relief and river/pond carving, so a
positive residual means locally low terrain rather than merely low global
altitude. It costs no neighbor samples. The same proxy modestly changes rain
particle count at the camera via one resident elevation lookup; open ocean
and snow keep the regional rate.

Overrides are accepted only as a valid whole. Non-finite values, zero/negative
denominators, inverted cover/snow bands, precipitation thresholds outside
`[0,1)`, layer counts outside 1..3, non-positive scales, densities/caps/biases
outside `[0,1]`, inverted cloud-shell heights, and particle counts above 100,000 emit
a warning and fall back to all defaults; malformed tuning can never feed NaNs
or an unbounded instance count to a frame.

The photo map's **Clouds now** layer reads the same on/off/pin state and
absolute weather time as the renderer. It samples the renderer's current
synoptic raster rather than evaluating a parallel per-map-pixel field; its UI
texture refreshes on that raster's deterministic two-second source key, while
seasonal-only bases retain their 60-second cadence. Pinned clouds are constant,
and weather-off intentionally draws an empty cloud overlay. The
Neisor-only **Wind** toggle integrates deterministic short streamlines through
the baked east/north wind plus the active analytic cyclone/zonal drift; comet
tails and heads remain legible at whole-planet and zoomed map scales.

## Phase roadmap

- **W1 (2026-07-09):** weather.bin bake + weather.rs field + sky cloud
  shell (from below/aloft, fades before orbit) + sun/ambient dimming +
  precip particles (rain streaks / snow flakes) + snow dusting and rain
  darkening in the shared shader + `weather` play command + sidecar
  fields + weather-visual.play demo suite.
- **W2:** cloud shadows on the ground (project the shell noise along the
  sun), fog banks / valley mist by humidity + dawn, storm-edge lighting
  (dark horizons, shafts), wind-driven particle slant.
- **Clouds v2 / W3 (2026-07-11):** three weather-typed 2-D shells at distinct
  altitudes fake depth through parallax; the same seeded formations composite
  over the planet from space with a hard post-stack opacity cap.
- **W2.5 (2026-07-13):** the heterogeneous deck. Layer-2 cover/precip are
  baked to a 6x64x64 edge-inclusive cube-face raster and sampled bilinearly at
  each shell hit (sub-texel world-stable dither hides interpolation contours).
  Regional storm belts/clear lanes now agree with Clouds-now from ground to
  orbit; existing fabric supplies the local scale. B-8's zero-cover cirrus
  inversion is gated in live mode so a clear synoptic lane actually clears,
  while pinned captures retain the established uniform law. Added the Neisor
  wind-streamline map layer and headless `weathermap` evidence exporter.
- **W4 (design settled 2026-07-12, below):** structural seasonality —
  frozen lakes melt in summer, sea ice advances/retreats (seaice_monthly
  is already baked per month!), snow line moves in sample(), deciduous
  tree color cycles. Andrew's D-9 verdict: "Water should generate
  variably as either frozen or not based on the season according to
  temperature and weather statistics, and should update when revisited.
  Manually placed ice or water should only break this rule if
  temperature warming or cooling systems exist."

  THE W4 DESIGN CONTRACT:
  - ONE INPUT: season_frac (Neisor mean anomaly at absolute T_S - the
    orbital clock P1 unified). Every seasonal decision reads the baked
    k=2 Fourier temperature AT THE SEASON, through one shared function
    (seasonal_temp_c(pos, season_frac)); nothing invents a second
    seasonal model. Weather-off keeps today's annual-mean behavior.
  - CLASS RULE: the frozen/liquid decision (today: temp_c < -4)
    becomes seasonal_temp_c < FREEZE_C with a HYSTERESIS BAND (freeze
    below -5, thaw above -2, say) so shores do not flicker classes at
    the contour; band edges dithered by the ecotone comparator like
    every other boundary in this codebase.
  - REVISIT SEMANTICS COME FREE: columns/chunks/tiles are pure
    functions of (position, seed, T_S) - a chunk rebuilt on revisit
    simply evaluates the current season. No stored melt state, no
    migration. Loaded-but-stale chunks re-stream when their season
    class flips (cheap check: chunk carries the season bucket it was
    built at; the streamer refreshes on bucket change - buckets
    quantize season_frac so mid-season play never rebuilds).
  - EDITS WIN: player-placed blocks override seasonal class (existing
    per-body edit machinery already does this).
	\[This is an okay consequence for now, but realistically, player-
	placed blocks should follow the same rules according to
	temperature. With this behavior, incidental behavior like picking
	up a bucket of water, digging for sand or shoal on the coast, or
	creating an irrigation channel would unrealistically override the
	expected natural phenomena. Again, if block temperature is ever
	implemented this can change according to player-made or natural
	heating or cooling mechanisms.\]
  - PHYSICS FOLLOWS CLASS: walkable ice in winter, swimmable water in
    summer, at the same coordinates. The census/colcensus/sync_diff
    instruments and every W1-W8 invariant hold AT EVERY SEASON.
  - SUITE TIME MODEL: all existing suites pin the CANONICAL SEASON
    (weather season 0.45 - today's epoch, byte-compatible with every
    blessed baseline). NEW seasonal suites sweep season_frac at fixed
    poses: a winter-walk assert (grounded on ice where summer swims),
    a melt assert (swimming where winter walked), reel poses at
    winter/summer solstice for frozen_lake and sea_ice, census cohort
    parity at 4 season points (totals may move; W1/W2/W5 must not).
  - SNOW LINE + DECIDUOUS: the snow override and grass/canopy tints
    read the same seasonal_temp_c; the vegetation field itself (tree
    positions) stays season-independent (trees do not teleport).
- **W5+ (dream list):** lightning + thunder delay, rainbows (sun-opposite
  arc when sunny+rain), aurora at high latitude night, dust storms in
  deserts (precip=0 + high wind), wind-swayed trees/shrubs, puddle
  accumulation on flats, snow depth as voxel overlay, weather-aware
  ambient audio.
- **W4.1 (roadmap, Austin 2026-07-13):** smoother seasonal landscape
  transitions - the 24/year season buckets step visibly under time
  fast-forward ("make the increments smaller"). Directions: raise the
  bucket count (cheap, more rebuild churn), or interpolate the frozen
  class per column across the bucket boundary via the existing
  hysteresis band + comparator dither so refreshes land as advancing
  fronts rather than steps. Verified context: seasons work, ice
  dynamics read plausibly at speed.
- **W-MOTION (banked 2026-07-12, Austin: "distortions in the field that
  evolve over time... full turbulence as the ceiling", everything
  deterministic and O(1) in t):** true turbulence needs path-dependent
  transport (integration, state) and is off the table by the time-travel
  constraint - but its LOOK is reachable by composing closed-form terms,
  every one a pure f(position, t, seed). Ordered cheap-to-flagship:
  1. PHASE-VELOCITY OCTAVES: each fabric octave drifts at its own speed
     and heading (fine octaves faster - an energy-cascade feel). Zero
     extra noise taps; kills the rigid-sheet read of the deck.
  2. EVOLVING DOMAIN WARP: sample position warped by a slow vector
     noise that itself drifts, p' = p + A*warp(p*fw - u*t). Two taps.
     Filaments stretch and fold - the visual signature of advection.
  3. DIFFERENTIAL ROTATION: Rodrigues-rotate the domain about the spin
     axis by theta(lat, t) = (w0 + w1*cos^2 lat)*t - Jupiter-style
     zonal shear, features at different latitudes slide past each
     other. One rotation per pixel, closed form.
  4. ANALYTIC VORTICES (the flagship): N seeded cyclones, each a
     closed-form swirl in its co-moving frame (center rides a zonal
     track c_i(t); local domain rotates by exp(-d^2/r^2)*w*t).
     Placement/intensity modulated by the baked storm field so
     cyclones live where the bake says storms live. Rotating storms
     with eyes, visible from orbit.
  5. FRONTS: ridged line features on the phase-velocity scheme with
     asymmetric (sharp leading, trailing smear) profiles.
  6. 4D NOISE: t as a fourth hash dimension at a slow rate - cells are
     born and die in place, complementing all transport terms above.
  All terms compose in the planet-anchored domain (see the 2576050
  drift fix - camera-anchored motion is the one forbidden shape) and
  none touch weather replay, sidecar restore, or double-run byte
  determinism. Natural mission size: 1+2+6 first (pure look), then
  3+4+5 (structure) with an orbital reel pose per term.
  \[All agreed upon. Time-dependency and all of the behavior listed
  above are both things we'd like to add. Speaking of "Jupiter-style
  zonal sheer, a similar type of simulation would be an awesome touch
  to stars and gas giants.\]

## The bounded-shear law (2026-07-13, from Andrew's "Cyclone Evolution" photos)

Any domain-warp term of the form rate*t with a SPATIALLY VARYING
coefficient accumulates unbounded differential shear and eventually
strips the cloud fabric into threads. Two terms had this disease and
both are now bounded; treat this as a standing rule for every future
W-MOTION term (a rigid whole-sphere rotation is the only safe
unbounded motion):

- CYCLONE WRAP: was exp(-r^2)*w*t (208 fabric turns by t=100000 s).
  Now each system carries a per-life bounded wrap angle,
  max_wrap*tanh(rate*age/max_wrap), computed once on the CPU and
  consumed identically by the CPU field and the GPU uniform
  (cyclone_fronts.w). The fabric tolerates about 2 radians of core
  twist before tangential stretch (~2*wrap*rn^2*exp(-rn^2)) reads as
  concentric threads - hence cyclone_max_wrap_deg = 120 with the
  eye/eyewall/envelope loads carrying the storm shape. 480 exactly
  reproduces Andrew's photos; keep tuning under ~200.
- CYCLONE LIFECYCLE: storms are finite now - cyclone_life_s (6300 s
  = 3.5 game days) with a smooth grow(18%)/mature/decay(22%)
  envelope on intensity, phase-staggered per system index, and each
  life re-rolls its seeded corridor (epoch-salted candidate tracks),
  so long clocks see storms born, wander, and die across the storm
  belts instead of two eternal vortices winding forever. Age is a
  pure function of (t, seed, index): a time-travel seek lands
  mid-life correctly, O(1).
- ZONAL SHEAR: theta = (w0 + w1*cos^2 lat)*t stretched the whole
  deck into east-west threads by t~100000 s (28 game hours - minutes
  of wall time at 600x). The w1 term is now a bounded slosh,
  A*sin(2*pi*t/P) with A = rate*P/(2*pi) (zonal_shear_period_s,
  default 14400 s), so bands slide back and forth (QBO-style) with
  the configured rate but capped displacement. The w0 base term is
  rigid rotation and stays proportional to t.

## Spiral-arm density wave (2026-07-13, Andrew's design)

Confirmed intent first: a mature cyclone does NOT rotate its fabric -
the twist is tanh-held, so cells seed and disperse in a statically
spiraled domain (true rotation only during spin-up). Andrew's
refinement, implemented: cover is DENSER along logarithmic spiral
arms whose pattern rotates with storm age while the fabric stays
put - a density wave, exactly how galactic spiral arms work. A
rotating density mask is rigid in angle-space, so it accumulates
zero shear and may rotate forever without re-creating the streak
bug; the phase is a pure function of age (seek-safe), and it dies
with the life envelope. Same signed-sharpened cosine term on the
CPU (structured_loads) and GPU (cloud_structure), arms gated
outside the eyewall. Knobs: cyclone_arm_count (2; 0 disables),
cyclone_arm_strength (0.55), cyclone_arm_twist (3.0),
cyclone_arm_spin_deg_s (1.2). VISIBILITY LAW: the arms visibly
revolve at spin/arm_count - the first default (0.35) meant one
revolution per ~34 min of weather time, hour-hand territory, and
Austin+Andrew correctly reported "no rotation" at 1x. At 1.2 a
revolution takes 10 weather-minutes: majestic at 1x, a clear sweep
at 10-60x. The knob panel now ships as
viewer/assets/weather_tuning.json (tracked; overrides any subset,
ignored by the multiplayer identity handshake). Perf note: the moon_orbit
perf-reel p95 rose 4.4 -> 6.1 ms because the harness bakes the
synoptic raster synchronously on its settle path and the planetary
wave added two octaves per texel; the live game bakes async on
rayon, so no gameplay frame pays this. Baseline re-blessed.

## Global cover variance (same session, Austin's ask 2)

Cover was climatology + synoptic(40) + meso(320) - nothing below
synoptic scale, so global cover averaged out homogeneous. A
PLANETARY WAVE band now sits under the stack: fbm_band(2 octaves,
planetary_freq = 5 ~ hemisphere-scale) advected at 0.55x the
synoptic drift, amplitude planetary_strength = 0.32 in raw-cover
units. Continent-sized breaks and dense belts, CPU-only (rides the
W2.5 raster into the deck; no shader change). Answer to Austin's
question: cover IS driven by the baked cloud/precip climatology
(WEA3), which gives the geographic pattern - the planetary wave adds
the missing largest-scale variance on top.

## Decision points for Andrew

Moved to DECISIONS.md (repo root). D-6 and D-8's 2026-07-11 verdicts are the
clouds-v2 and crevice interpolation implemented above; D-5, D-7, and D-9
remain tuning/future-phase context.

## Data format: weather.bin

```
magic  b"WEA3"
u32    res (texels per face edge, 256)
u32    n_layers (26)
6 faces x n_layers x res^2 f32, v-major rows, layer order:
  temp_a_c, temp_b_c        # temp mean lives in face_*.bin already
  prc_mean, prc_a1, prc_b1, prc_a2, prc_b2  # mm/month
  cld_mean, cld_a1, cld_b1, cld_a2, cld_b2  # cloud fraction 0..1
  wind_e, wind_n            # m/s annual mean
  seaice_month_0..11        # monthly sea-ice truth, spatially filtered
value(t_yr) = mean
            + a1*cos(2*pi*t_yr) + b1*sin(2*pi*t_yr)
            + a2*cos(4*pi*t_yr) + b2*sin(4*pi*t_yr), t_yr in [0,1)
```

Temperature uses only the annual pair (its mean is in face_*.bin); the k=2
line applies to precipitation and clouds. Its shipped k=1 residual is 0.63 C
against a 9.35 C mean seasonal swing, so two more always-sampled rasters were
not justified for this targeted bimodal-weather fix.

W4 samples the temperature pair through `seasonal_temp_c` and linearly
interpolates the twelve sea-ice layers between month centers. Sea ownership
always uses the monthly raster, even where it disagrees with the Fourier air
temperature; inland water uses the temperature hysteresis law. A visible
coastal class seam is therefore possible at a disagreement, by design: the
baked ocean state is authoritative on the sea side.

The fit uses month centers at `(m+0.5)/12`. CARTESIAN coefficients are on purpose:
amp/phase cannot be blended across texels (phase wraps — a texel between
a January-peak and a December-peak cell would average to July).

Rasters are edge-inclusive like face_*.bin (texel 0 and res-1 on the cube
edge) and sampled bilinearly; weather varies at synoptic scales, so 256
per face (~40 km/texel) is deliberately coarse and cheap (22,020,108 bytes,
about 21 MiB). The loader accepts legacy WEA1/10-layer files loudly by
zero-filling the missing k=2 terms; corrupt/unknown versions fail gracefully
and name the rebake command.
