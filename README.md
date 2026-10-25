# Triangulum

A planetary-scale voxel game, built by a father and son. One seed grows an
entire earthlike world — plate tectonics, climate, rivers, biomes, a creation
chronicle — and then you fly down from orbit and walk on it, one-meter blocks
under your feet, on the same planet the maps describe.

What lives in this repo:

* **`planetgen/`** — the planetary map generator (Python): tectonics,
  climate, hydrology, Köppen biomes, named geography. Outputs the
  game-facing dataset and an atlas of map sheets. See `planetgen/DESIGN.md`.
* **`game/`** — the new engine (Rust + wgpu), under construction since
  October 2026: a 60x-smaller Neisor with real 3D blocks, voxel LODs from
  your feet to orbit, and (soon) water and sand physics.
* **`crates/cubesphere/`** — the equal-area cube-sphere grid both use.
* **`viewer/`** — the July 2026 viewer, frozen. It still runs; see
  `viewer/README.md`.

Start with `PLAN.md` and `STATUS.md`. Design decisions are in `DESIGN.md`.

## Getting started from scratch

You need **Python 3.11+** and **Rust** (stable). One-time setup:

1. **Python** — install from <https://www.python.org/downloads/> (check
   "Add python.exe to PATH" on Windows), then:
   ```
   pip install numpy scipy matplotlib numba
   ```
2. **Rust** — install rustup from <https://rustup.rs>. On Windows pick the
   default MSVC toolchain (it will offer to install the Visual Studio C++
   build tools if you don't have them). `cargo --version` should work in a
   fresh terminal afterwards.

Then generate the planet and fly it in the new engine (from the repo root):

```
# 1. grow the planet once (~1.5 h on the CPU; skip if output/seed42_r8 exists)
python -m planetgen --seed 42 --res 8

# 2. bake it for the engine (~20 s + ~1 min; writes game/assets/world/seed42)
python scripts/bake_world.py
python scripts/bake_rivers_v2.py

# 3. fly (the first build takes a few minutes)
cd game
cargo run --release
```

`STATUS.md` lists the current controls and the options for trying other
vertical scales and block sizes. The July viewer still runs from `viewer/`;
its own bootstrap (`bake_faces.py`, `bake_rivers.py`, `bake_weather.py`) is
in `viewer/README.md`.

## Planetgen quick start

Requires Python 3.11+ with `numpy`, `scipy`, `matplotlib`.

```
python -m planetgen --seed 42 --res 6
```

### Making it fast (optional accelerators)

```
pip install numba                  # ~20-40x on the erosion inner loops
pip install "cupy-cuda12x[ctk]"    # NVIDIA GPUs: climate stage on the GPU
python -m planetgen --seed 42 --res 8 --gpu
```

Both are optional and auto-detected; without them everything runs on plain
numpy. numba results are **bit-identical** to the pure-Python path. `--gpu`
runs the climate stage (≈80% of generation time) in float32 on the GPU —
fields match the CPU within float32 tolerance, but the planet is not
guaranteed bit-identical across machines, so treat CPU as the canonical
same-seed-same-planet reference. The first `--gpu` run compiles GPU kernels
(a one-time ~1 min cost, cached on disk).

~2.5 minutes later, look in `output/seed42_r6/`:

| file | what |
|---|---|
| `maps/atlas.html` | all map sheets with a layer switcher (open in a browser) |
| `maps/01_relief.png` … `10_globes.png` | individual map sheets |
| `CHRONICLE.md` | the planet's generated creation story |
| `planet_data.npz` | every field as cell arrays (the game-facing dataset) |
| `planet.json` | names, features, Köppen legend, config echo |
| `cache/` | per-stage caches for `--from` |

## Common workflows

```
# a different world
python -m planetgen --seed 7

# higher resolution (163,842 cells, ~82 km spacing; ~17 min)
python -m planetgen --seed 42 --res 7

# showcase resolution (655,362 cells, ~41 km spacing; ~1.5 h)
python -m planetgen --seed 42 --res 8

# tweak a parameter and re-run only from that stage (uses cache before it)
python -m planetgen --seed 42 --set erosion_iters=100 --from hydrology
python -m planetgen --seed 42 --set rain_orographic=6.0 --from climate2

# stop early while iterating
python -m planetgen --seed 42 --until hydrology
```

Stage order: `tectonics → climate1 → hydrology → climate2 → biomes →
chronicle → render`. Any config field can be overridden with repeated
`--set key=value`; see `planetgen/config.py` for the full tunable list with
comments — it is the intended "control panel" for experimentation.

## Watching the simulations run

```
# capture frames of every simulation as it runs
python -m planetgen --seed 42 --record

# ...and also open a live window during the run
python -m planetgen --seed 42 --watch

# ...and also assemble mp4 clips (requires ffmpeg on PATH)
python -m planetgen --seed 42 --record --video
```

`--record` writes `<out>/simviz/player.html` — open it in a browser and scrub
or play through each sequence: tectonic construction step by step, ocean
currents relaxing into gyres, moisture blowing off the oceans with the rain
belt sweeping through the seasons (`climateN-moisture`, `climateN-rainfall`),
and mountains eroding while rivers organize (`hydrology-elevation`,
`hydrology-discharge`). Space = play/pause, arrow keys = step. Frame cadence
and size: `--set record_every=2`, `--set record_width=960`.

## Parameters worth playing with first

| knob | effect |
|---|---|
| `grid_kind`, `grid_jitter` | sphere discretization: `fibonacci` (default) or `icosphere` |
| `n_plates`, `plate_warp_amp` | continent count & coastline wiggle |
| `continental_fraction`, `ocean_fraction` | land/sea balance |
| `orogen_amp_km`, `orogen_width_km` | how mighty collision ranges get |
| `n_hotspots` | volcanic island chains |
| `axial_tilt_deg` | seasons (try 0 or 40!) |
| `itcz_amplitude_deg`, `itcz_land_boost`, `thermal_wind_ms` | rain-belt migration & monsoon strength |
| `moisture_diffusion` | rain-field smoothness vs. streaky texture |
| `target_land_precip_mm` | overall wetness |
| `erosion_iters`, `erode_k` | how deeply rivers carve |
| `subsidence_total_km` | big rift lakes and inland seas |

## Reading the dataset from game code

```python
import numpy as np
d = np.load("output/seed42_r6/planet_data.npz")
# cell arrays: elevation_km, is_ocean, koppen, temp_c_monthly (12,N), river ...
# topology:    xyz (N,3 unit vectors), neighbors (N,6, -1 padded), receiver
```

Cells are vertices of a subdivided icosahedron. To sample at an arbitrary
lat/lon, build a KD-tree on `xyz` and inverse-distance-weight the k nearest
cells (see `planetgen/grid.py:sample_latlon` for reference code). All noise
in `planetgen/noise.py` is a pure function of 3D position and seed, so local
generation can re-evaluate it at any resolution for consistent fine detail.

## License

MIT — see `LICENSE`. This game is free and will stay free: anyone can play
it, learn from it, and build on it. We keep the option of someday selling an
expanded edition built on top of it; the version published here remains free
either way. Contributions are welcome under the same license.
