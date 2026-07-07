# Triangulum planet generator — design notes

The pipeline turns one integer seed into an earthlike planet: terrain, climate,
rivers, biomes, named geography, and a creation chronicle. It is the *high-level
map* stage of the game — its output seeds local voxel generation later, and
deliberately encodes no voxel arrangement or engine choice.

```
seed ──> tectonics ──> climate #1 ──> hydrology ──> climate #2 ──> biomes ──> chronicle ──> maps + dataset
```

## Design principles

* **Stylized simulation, not physics.** Every stage uses the simplest model
  that produces the *pattern* real geography has (gyres, monsoons, rain
  shadows, endorheic basins), calibrated against Earth numbers. Nothing here
  would survive peer review; everything here survives being looked at.
* **One seed, deterministic everywhere.** Each stage derives its RNG stream
  from `(seed, stage)`, so tweaking climate parameters never reshuffles the
  continents.
* **Resolution independence.** All noise is 3D and evaluated on the unit
  sphere; the game can re-evaluate the same functions at finer scales, and the
  dataset ships interpolation-ready cell data instead of raster images.
* **Stage caching.** Every stage's output is cached; `--from hydrology`
  re-runs only from there. Tuning loops stay minutes, not hours.

## The grid

A **Fibonacci-Delaunay sphere grid** (the Voronoi dual of a golden-angle
spiral lattice): 40,962 cells at level 6 (~165 km spacing), 163,842 at level
7, 655,362 at level 8. Points are laid on a Fibonacci spiral, lightly
jittered (0.12 of the cell spacing — enough to break the spiral's zonal
rings, which otherwise stripe advected fields), and triangulated by their
convex hull, which on a sphere *is* the spherical Delaunay triangulation.
Unlike a subdivided icosahedron there is no global structure — no faces, no
preferred directions — so the grid cannot imprint seams or bands on the
simulations. Cell degree varies (mostly 6, some 5/7); all operators handle
variable degree. A subdivided icosahedron (`grid_kind="icosphere"`, prefers
jitter ~0.35) is kept for comparison. `grid.py` supplies the graph operators
the stages share: least-squares gradients, Laplacian smoothing, divergence, two upwind
advection schemes (conservative for moisture — convergence zones *should*
accumulate; bounded/interpolating for SST — temperature must never overshoot),
multi-source distance transforms with nearest-source labels, and KD-tree
samplers for rendering and for arbitrary lat/lon queries.

## Stage 1 — tectonics (`tectonics.py`)

* ~11 plates grown as *noise-warped Voronoi* regions around farthest-point
  seeds (organic boundaries, varied sizes); the largest plates split into
  subplates with perturbed motion.
* Every (sub)plate rotates about a random **Euler pole**. Relative velocity
  across each boundary edge classifies it convergent / divergent / transform
  *with a rate* — feature amplitudes scale with collision speed.
* Continental crust ("craton") is assigned per plate and biased toward plate
  interiors, so passive margins and shelf seas emerge; total continental area
  targets ~40% of the surface, of which roughly a third floods.
* Boundary features, by crust pairing: continent–continent → wide orogens +
  plateau; ocean–continent → offshore trench + volcanic cordillera ~190 km
  inland; ocean–ocean → trench + island arc (subduction polarity chosen
  deterministically per plate pair); continental divergence → rift valley
  with shoulders; oceanic divergence → mid-ocean ridge, expressed through
  **crust age**: distance from the ridge over the half-spreading rate gives
  age, and depth = −(2.55 + 0.24·√age) km, the real subsidence law.
* **Deep history:** 1–2 entirely separate *ancient* plate layouts are
  generated; their collision zones are stamped onto today's continents as
  low, wide, eroded ranges (Appalachians/Urals analogs).
* **Hotspots:** fixed mantle plumes trail island chains along each plate's
  motion (Hawaii), decaying with age; continental plumes make basalt plateaus.
* Fractal noise is amplitude-modulated by tectonic context (ridged noise in
  active orogens, gentle fBm on cratons, fine grit on the abyssal plain), all
  domain-warped to kill any grid feel.
* Sea level is solved so exactly the target fraction (71%) floods, then the
  extremes are tanh-compressed toward Earth limits (~8.8 km / −11 km).
* Extra outputs for later stages: active **uplift** (ridge-noise-modulated so
  re-uplift builds carvable structure), **subsidence** (rift floors and
  cratonic sags — this is what keeps big lakes alive through the erosion
  stage), erodibility, volcanic mask, seafloor age.

## Stage 2 — climate (`climate.py`), run twice

Twelve monthly snapshots.

* **Insolation:** analytic daily-mean top-of-atmosphere flux from axial tilt.
* **Temperature:** annual structure uses cos-power latitude profiles (the
  shape heat transport actually produces; the ocean's curve is flatter and
  bottoms at −6 °C because currents keep polar seas near freezing). The
  *seasonal swing* comes from monthly insolation anomalies, damped by
  **maritime influence** — a field advected inland from the ocean along the
  prevailing winds, so west coasts are mild and continental interiors swing
  hard. Ocean seasons lag a month. Altitude applies a 6 °C/km lapse.
* **Winds:** seasonally migrating circulation bands (trades, westerlies,
  polar easterlies; the ITCZ wanders ±9° over ocean and up to ~2.3× that over
  land longitudes — land heats and cools fast, so the rain belt chases the
  summer hemisphere much farther across continents, per-longitude scaled by
  tropical land fraction) plus a thermal component: flow toward warm
  anomalies, rotated ~58° for Coriolis, capped — which is exactly a monsoon
  when a summer continent heats up.
* **Ocean currents:** relaxation of wind stress with coastline steering
  (no-normal-flow), neighbor smoothing, and mild divergence damping →
  subtropical gyres with boundary currents. SST is then advected along the
  currents with the *bounded* scheme, warming east-coast poleward currents
  and cooling west-coast equatorward ones; coastal temperatures inherit the
  anomalies through the maritime blend.
* **Moisture:** evaporation (Clausius-Clapeyron-shaped, capped) → conservative
  upwind advection by the wind field, plus a small isotropic diffusion per
  step (the upwind scheme's own diffusion is directional and would otherwise
  draw streaks along the trades that meet in chevrons at the ITCZ) → rainfall
  from four processes:
  convective (warm + convergent air, i.e. the ITCZ, further scaled by the
  **SST anomaly** — the rain band flares over warm pools and breaks over cold
  currents instead of being uniform along its length), orographic (wind ·
  land slope — rain shadows fall out for free), frontal (mid-latitude band ×
  wind speed), and saturation excess (cold air wringing out). The seasonal
  phase advances *continuously inside* the moisture loop (band winds and
  their convergence are analytic, so they evaluate at fractional months) —
  otherwise the rain belt sits at 12 discrete positions and stripes the
  annual map. Precipitation units are calibrated at the end so global land
  mean hits a target (830 mm/yr default).
* Collected: monthly T, P, winds, cloud cover, snow, SST, sea ice; annual
  currents, PET.

The stage runs once on tectonic terrain (to get rainfall for erosion) and
again on the carved terrain with lakes as extra moisture sources.

## Stage 3 — hydrology & erosion (`hydrology.py`)

* Runoff = precip − 0.7·min(PET, precip), floored at 2% of precip.
* Each of ~120 iterations: **priority-flood** depression filling (Barnes
  algorithm; every land cell drains), downhill receivers with **valley
  capture** (prefer neighbors already carrying flow) and fixed seeded
  **meander weights** (winding channels instead of lattice-straight ones),
  downstream discharge accumulation, then
  **stream-power incision** (E ∝ erodibility · Q^0.5 · S, capped per step),
  **sediment transport** — eroded volume rides downstream, deposits where
  capacity (∝ Q^0.5·S) drops: floodplains, basin fills, and coastal deltas —
  **slope-gated hillslope diffusion** (steep faces shed mass like talus;
  gentle terrain keeps its grain instead of blurring away — this is what lets
  the landscape get *rougher* over time, not smoother), continued **uplift**
  in active orogens (two-scale ridged modulation, so re-uplift builds
  dissectable ridge systems) and **subsidence** in rifts/sags. Frozen ground
  erodes at quarter rate. Late in the run, **ridge-only bedrock texture** is
  injected on thin-soil uplands (positive crests — signed texture would mint
  closed basins and speckle the world with ponds); the final steps route
  rivers through it. A roughness metric (mean local relief, seed vs final)
  prints every run — current settings come out slightly *above* seed
  roughness, matching the design goal.
* Sub-grid "grit" noise is added up front, and routing is
  **discharge-attracted**: a cell prefers the downhill neighbor already
  carrying flow (cycle-safe — only strictly downhill candidates), so big
  channels capture their neighbors and streams merge into dendritic trees
  instead of running parallel down a regional slope.
* Sea level is re-solved afterwards (isostasy hand-wave) so the ocean
  fraction stays on target.
* Remaining depressions become **lakes**: if inflow beats evaporation at the
  spill level the lake overflows (fresh); otherwise the level drops until
  evaporation balances inflow (endorheic **salt** lake).
* Rivers = cells above a discharge threshold; the dataset stores the full
  receiver graph so the game can reconstruct every stream.
* Soil depth = deposited sediment + climate-dependent weathering, thinned on
  steep slopes.

## Stage 4 — biomes (`biomes.py`)

Full 30-class **Köppen-Geiger** classification (Peel et al. 2007 thresholds)
from monthly temperature and precipitation, plus game-facing layers:
vegetation density (Miami-model NPP), soil fertility (soil + NPP + volcanic +
floodplain bonuses), permafrost, ice caps.

## Stage 5 — chronicle (`chronicle.py`)

Connected-component analysis names the geography (continents, oceans, young
and ancient ranges, top rivers traced along the discharge-maximal stem,
lakes, deserts, hotspot archipelagos) with a seeded syllable generator, then
writes `CHRONICLE.md` — the planet's creation story with real numbers in it.

## Rendering & dataset (`render.py`, `pipeline.py`)

Ten map sheets (relief/rivers/ice, plates+motion, seasonal temperature,
precipitation, seasonal winds, currents over SST, Köppen, vegetation,
orthographic globes with cloud cover) plus a small `atlas.html` layer-switcher.
`planet_data.npz` carries every field as float32 cell arrays with the grid
topology, and `planet.json` carries names/features/config — everything a game
engine needs to sample the world at any resolution.

## Known simplifications (deliberate, revisit as needed)

* No glaciation model (no fjords, U-valleys, glacial lake fields, isostatic
  rebound); cold regions just erode slower.
* Currents are a relaxation sketch, not shallow-water flow; no ENSO-style
  variability, no tides.
* One erosion epoch at map scale; canyon-scale carving belongs to the local
  voxel stage.
* Rain calibration is global; regional water balance is only as good as the
  moisture advection sketch.
* Climate ignores CO₂/greenhouse variation, orbital eccentricity, and axial
  precession — the "paleoclimate story" is a single steady state.
