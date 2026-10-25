# Feature inventory: the July 2026 viewer

*Written 2026-10-25 for the M1-inventory task in PLAN.md. I read `viewer/`, the three bake scripts (`scripts/bake_faces.py`, `bake_rivers.py`, `bake_weather.py`), `planetgen/` and `docs/archive/`, and built or ran nothing. The viewer source last changed at commit `7892d90` (2026-07-26), and all line numbers are for that commit. Paths are relative to the repo root, `C:\code\triangulum`. **(inferred)** marks a conclusion reasoned from code or arithmetic rather than read directly. Colours are linear RGB unless noted; the old render target was `Rgba8UnormSrgb`.*

**Summary.** Most of the old world's character was generator logic, and that logic ports. This covers geological strata, the climate-tinted biome surfaces, trees and rocks, rivers, lakes and ponds, karst caves, the snowline, the moon's craters and maria, the stateless orbits, and the weather. A large share of the old code existed only to hide the boundary between the far mesh and the near voxels, and should be dropped on purpose rather than ported by accident. That includes far-biome reconstruction, geomorph, tree and rock impostors, rim lifts, a karst shader twin, and `sync_diff`. Almost every numeric constant needs re-deriving, because the old viewer's Neisor had a radius of **8,660 km**, not 6,371 km. Against what the old viewer drew, the new world is about 1/96 horizontally and 1/70 vertically, and its slopes are 1.36× steeper. The old game's **strata were never real blocks and never geology**. They were a fixed column (grass, then 3 dirt, then uniform stone) plus a subtle shader tint of 9 m beds on cliffs, using a 5-colour hashed palette. planetgen computes rock hardness, volcanic arcs, cratons and soil depth, but the bake drops all of them. Two items are urgent before any porting. First, **Andrew's own spec notes and reference images are gitignored** and exist only in this working tree (§0.5). Second, the 30-minute day Andrew set conflicts with DESIGN.md's 24 minutes (§0.6).

Every entry below gives:
- **Looks like**: what the player sees.
- **Where**: file and line ranges.
- **How**: enough to re-implement it.
- **Verdicts**: what Andrew or Austin said about it.
- **Port**: one of *as-is*, *rethink* (with how), or *drop*.
- **Milestone**: which milestone it belongs to, and why.

---

## 0. Read this before porting anything

### 0.1 Scale
- **Old radius.** The old viewer's Neisor radius was **8,660.254 km**: `scripts/bake_faces.py:97` sets a cube edge of 10,000 km, and `viewer/assets/meta.json` agrees. planetgen simulates at 6,371 km (`planetgen/config.py:22`). The old viewer stretched planetgen's data 1.36× horizontally and kept heights unscaled (exaggeration 1.0, `main.rs:67`).
- **Ratios to the new R = 90.57 km:**

  | Compared with | Horizontal | Vertical |
  |---|---|---|
  | the old viewer | 1/95.6 | 1/70.3 |
  | planetgen | 1/70.3 | 1/70.3 |

  Every slope will therefore look 1.36× steeper than the old viewer showed it, and every tuned slope threshold was tuned on the flatter version.
- **Old columns.** There were 10,000,000 gnomonic columns per face, about 1.73 × 1.0 m at a face centre. The moon had 2,700,000 (`voxel.rs:33-39`).

### 0.2 How to port a constant
1. **Noise on the unit direction.** Most noise is `noise(dir · f)`, so one lattice cell is R/f. If a constant is pasted unchanged, the feature shrinks 95.6×:

   | Field | f | Cell at old R | Same f at new R |
   |---|---|---|---|
   | terrain detail, octave 0 | 700 | 12.4 km | 129 m |
   | cave region gate | 90 | 96 km | 1.0 km |
   | cave tube n1 / n2 | 90,000 / 76,000 | 96 / 114 m | 1.0 / 1.2 m |
   | strata fold | 96 | 90 km | 943 m |
   | ground patchiness | 4,500 | 1.9 km | 20 m |
   | pond mask | 16,000 | 541 m | 5.7 m |

   Features tied to planetgen geography (relief, biome warp, storm belts) may keep `dir · f` and shrink with the planet. **Block-scale features (caves, beds, soil, rocks, cloud cells) must be restated in metres.**
2. **Earth-scale classification thresholds.** Examples are the beach band (9–16 m), coastal roughness damping, the rock "alpine" line at 1.45 km, the cave gate (ground above 4 m), lake depth cuts and the 6.5 °C/km lapse. Either evaluate them on unscaled height (h_new × 70.3), or restate them in blocks. Applied verbatim to 1/70 heights they break.
3. **Block-scale quantities** stay in metres or blocks: tree shapes, pond depth, the shore band, torch radius, particle sizes.
4. **Quantities in units of R** (orbit distances in Neisor radii, the fly-speed cap) rescale automatically if they are kept R-relative.

### 0.3 Camera up is no longer pixel up
At R = 8,660 km the camera's up vector was within 1.7° of every pixel's up inside 80 km, so the shader used `globals.sky.xyz` as every pixel's up. Uses include:
- the hemisphere ambient;
- the strata early reject;
- the day factor below 2.5 km;
- the snow-dusting lapse.

On a 90 km planet, 80 km of arc is about 50° **(inferred)**. All of these must become per-pixel.

### 0.4 Horizons shrink
Ground-horizon distances on the new planet **(inferred)**:

| Eye height | Horizon |
|---|---|
| 1.7 m | about 0.55 km |
| 100 m | about 4.3 km |
| 1 km | about 13.5 km |

Constants tuned for 100+ km sight lines must be re-derived: the fog density, the storm-edge probe, the impostor distance knots and the cyclone radius.

### 0.5 Preserve Andrew's source material (urgent)
`viewer/interchange/*` is gitignored (`.gitignore:13`). Only reel baselines, a few tint PNGs and `codex/W4-FINDINGS.md` are tracked. The files below exist **only in this working tree** and should be copied into a tracked folder; that is Austin's call.

- **`development-direction.txt`.** Andrew's long-range wish list:
  - Create- and Vintage Story-style engineering, with wires, pipes and branches embedded in blocks;
  - vehicles on their own grids;
  - a ½ m or ⅓ m voxel debate;
  - the "60× smaller world, 60× faster time, 60× efficiency" balance rule;
  - a real block palette with chunk saves only on change;
  - non-full blocks and block updates;
  - water levels with sloped meshes;
  - rivers at near-continuous height with flow in and out of the loaded area;
  - a block and texture editor;
  - a terrain-noise overhaul to remove the "texel" artefacts;
  - voxel LODs, so structures show at distance;
  - greedy meshing.
- **`solar-system-and-moon.txt`.** The moon generation spec and the camera spec (freecam, Q/E roll, LMB break / RMB place, body focus).
- **`moon-informal-bug-and-refinement-doc.txt`.** The **r3 maria and rays spec**, plus open moon bugs.
- **`requested-changes-and-bugs.txt`.** Camera requests C-1..C-3.
- **`notes-on-improved-terrain-gen.txt`.** Rivers perched above terrain; blue tiles on the coast.
- **Reference images:** `current-and-target-lunar-mare.JPG` (the blue-pen r3 acceptance sketch, which exists nowhere else), `tycho-rays-and-moon-surface.JPG`, `lunar-mare-color-consistency.jfif`, `current-moon.JPG`.
- **Codex findings:** `codex/SOLAR-P1/P2/P3-FINDINGS.md`, `MOONSCAPE*-FINDINGS.md`, `MOONLATTICE-FINDINGS.md`.
- **Evidence folders:** `strata/`, `geology/` and `lodcolor/` hold the only before/after contact sheets for strata, rocks and LOD tint.

### 0.6 Day length conflict
- The old code ships `day_length_s: 1800` (D-5, Andrew: "30 minutes per day, 7 days in a lunar cycle, 3 lunar cycles per season, 4 seasons per year … definitely adjustable"). DESIGN.md now says 24 minutes, i.e. 1,440 s.
- Orbit periods are stored in game days, so the 7-day moon and 84-day year carry over unchanged.
- **Confirm with Andrew** that 7 and 84 still stand under 24-minute days.

### 0.7 The determinism contract
"Everything visible is a pure function of (seed, position, absolute time), and edits are journaled" (viewer/CLAUDE.md). The harness, screenshot sidecars, reels, recordings and multiplayer all rest on it. Keep it as a contract in the new engine, and extend it to the water simulation: replays are a snapshot plus ordered inputs.

### 0.8 planetgen data the old viewer never used
- **Exported in `planet_data.npz` but never baked** (`pipeline.py:147-177`): `soil_km`, `plate`, `craton`, `seafloor_age_myr`, `vegetation` (NPP), `fertility`, `permafrost`, `current_e/n`, `sst_monthly`, and `wind_e/n_monthly`. Wind reached only the weather bake.
- **Computed but not exported** (`tectonics.py:412-416`): `hardness`/`erodibility`, `volcanic` and `uplift`.
- **`output/seed42_r8/CHRONICLE.md`** holds deterministic names for continents, ranges, peaks, rivers and oceans. They could become map labels (Later).
- These are the natural inputs for real strata, soil depth, karst density and dunes (§4.4).

---

## 1. World data pipeline (bakes)

### 1.1 Face rasters (`face_*.bin`)
- **Looks like:** the planet map the old viewer's LOD tree tops out at.
- **Where:** `scripts/bake_faces.py:44-104`; loader `planet.rs:1139-1225`; bilinear sampler `planet.rs:1227-1284`.
- **How:**
  - Six 1024² **edge-inclusive** rasters (`linspace(-1, 1, res)`), so neighbouring faces agree on shared edges.
  - Per texel, 21 bytes: elevation f32, Köppen u8 (255 = ocean), rough f32 (mean \|Δelev\| to graph neighbours), annual precipitation f32, annual mean temperature f32, `log10(1 + flow)` f32.
  - Continuous layers use inverse-distance weighting (power 2) over the 3 nearest cells; Köppen uses the nearest cell; flow uses the max of the 3 nearest ("keep river lines fat").
  - At runtime everything is plain bilinear; one texel is about 16.9 km at a face centre.
  - The ocean mask is blurred (radius 2) and re-derived across cube seams (`planet.rs:1186-1192`, the F-17 fix).
- **Verdicts:** DESIGN.md: "Sampling is smooth everywhere: no visible raster texels or noise seams." Andrew's development note asks for the "texel" artefacts to go.
- **Port:** rethink.
  - Bilinear interpolation over an IDW field creases slopes at texel edges and leaves flat spots at data points **(inferred)**. The old viewer hid both under kilometres of detail noise; at 1/70 relief they will show. Use a C1/C2 interpolant.
  - Keep edge-inclusive texels, nearest-class plus dither for categories, max-filtered flow, and seam-exact masks.
  - Add the §0.8 layers.
- **Milestone:** M1 (M1-bake), because generator v1 samples it.

### 1.2 River and lake bake (`rivers.bin`): see §8.
### 1.3 Weather climatology bake (`weather.bin`): see §12.1.

---

## 2. Terrain shape and detail

### 2.1 Sub-raster relief: roughness-driven ridged plus fbm
- **Looks like:** hills and ridges finer than the map. Rugged country is ridged; calm plains stay calm; valleys and lake beds flatten.
- **Where:** `terrain.rs:716-767`; constants `terrain.rs:27-28, 243-248`; noise `noise.rs:88-162` (`gradient_noise`, `fbm_band`, `ridged_band`). The Rust noise is a bit-exact port of `planetgen/noise.py`, with golden values in `tests/golden.rs`.
- **How:**
  - Coastal damping: `rough_r = rough · smoothstep(0.02, 0.30, e_raw_km)`. Without it, roughness spikes on continental slopes made kilometre-high coastal cliffs.
  - Envelope: `env = clamp(0.06 + 0.85·rough_r + 0.10·e_raw, 0.05, 1.7) · valley` (km).
    - `valley = 0.12 + 0.88·smoothstep(hw + 0.10, 5hw + 0.9, river_dist)`.
    - Lake beds: `env *= 1 − 0.88·smoothstep(−0.005, 0.012, level − e_raw)`.
  - Ridged weight `rw = clamp(0.30 + 0.50·rough_r, 0.30, 0.72)`.
  - `detail = rw·ridged_band(dir, 0, n, 700, seed+1013) + (0.95 − rw)·fbm_band(dir, 0, n, 700, seed+2027)`.
  - Each octave halves amplitude and doubles frequency.
    - Ridged term per octave: `amp·((1 − |n|)² − 0.5)`.
    - Gradient noise is Perlin-style with quintic fade, ×1.9.
  - Octave counts: voxels used 12 (finest lattice about 6 m); the mesh up to 8.
  - Floors:
    - `h = max(e_raw + env·detail, h_floor)`, with `h_floor = 2 m` where ocean fraction > 0.02, else −9 km.
    - The near-field sea floor was clamped to −6 m (`terrain.rs:614-624`).
- **Verdicts:** DESIGN.md: "Relief finer than planetgen's resolution comes from the engine's generator. Its roughness is a dial for Andrew."
- **Port:** rethink for 1/70.
  - Keep the structure: roughness envelope, coastal damping, floodplain and lake flattening, ridged/fbm mix by roughness.
  - `dir·700` happens to land well (a 129 m cell, about 1/3 of a scaled planetgen cell).
  - Amplitudes scale by 1/70: 1.7 km becomes 24 m and plains about 1.5 m. Octaves below roughly the 7th become sub-block **(inferred)**.
  - Andrew's roughness dial is an overall multiplier on `env`, maybe with a floor in blocks.
  - Drop the −6 m sea-floor clamp; the new world has about 150 m of real bathymetry. The 2 m coastal floor becomes 1 block.
- **Milestone:** M1 (generator v1); the dial goes in the M3 look panel.

### 2.2 Band-limited detail per LOD
- **Looks like:** coarse terrain that is the average of the fine terrain, so nothing pops on approach.
- **Where:** `terrain.rs:210-224` (`octave_count`: keep octave o while its wavelength is at least 2.5 × the sample spacing); `RIVER_REF_OCTAVES = 8` at `terrain.rs:245-248`.
- **How:** each LOD evaluates only the octaves it can represent. Wet/dry decisions always read terrain at a fixed octave depth, so every LOD agrees.
- **Port:** keep the principle for voxel LODs up to H: generate coarse voxels from the band-limited field, never by point-sampling the finest field. **Rule: no categorical decision may depend on LOD octave depth.**
- **Milestone:** M1 (LOD up to H).

### 2.3 Gentle-slope terracing
- **Looks like:** on gentle slopes, 1 m column tops make contour rings, for example every 20.6 m on a 2.8° slope (world-reel pose `gentle_terrace`, 8.097 −35.673).
- **Where:** a property of the lattice, not code (BUGS B-20).
- **Verdicts:** certified not-a-bug; "parked for Andrew as an art question only".
- **Port:** inherent to 1 m blocks. The ½ m switch halves it, and slope-lit top normals (§10.5) remove the shading half. Expect it to be more common now that slopes are 1.36× steeper.
- **Milestone:** M1 (it appears immediately); the verdict goes with the ½ m decision after M1.

### 2.4 Landforms that were planned but never built
- **Status:** none of these shipped. LANDSCAPE.md Pass 4 listed mesas, buttes, hoodoos, sea cliffs, sea stacks and scree/talus, then arches and overhangs once a 3D carve field existed. Also banked: wind-aligned dunes from the baked wind field, salt flats on endorheic basins, eroded Neisor meteor craters, glacier tongues, tidal flats and reefs. There is no cliff generator: cliffs are steep detail noise, which bares rock (§3.2).
- **Port:** real 3D blocks make arches and overhangs straightforward. The causal ideas are worth keeping: dunes aligned to `wind_e/n`, playas on endorheic basins, caprock benches from erodibility.
- **Milestone:** Later. Scree and talus could ride M2's falling blocks.

---

## 3. Surface materials and biome colours

### 3.1 Material table
- **Where:** `voxel.rs:101-154` (`Mat`, `Mat::color`).
- **How:** there were no stable block ids. The enum order was the index, and saves stored column height deltas, not materials.

  | Material | Colour | Note |
  |---|---|---|
  | Grass, Sand, Snow | climate tint (§3.4) | tinted per column |
  | Dirt | [0.33, 0.235, 0.135] | "dry loam"; raised from [0.23, 0.15, 0.085], which read as black holes at steps |
  | Gravel | [0.32, 0.31, 0.29] | |
  | Stone | [0.30, 0.29, 0.28] | |
  | Rock | [0.20, 0.195, 0.19] | dark bare rock |
  | Ice | [0.60, 0.72, 0.85] | frozen water |
  | Water | [0.055, 0.17, 0.30] | replaced by the depth ramp (§8.3) |
  | Log | [0.16, 0.10, 0.05] | |
  | LeavesBroad | [0.065, 0.20, 0.035] | |
  | LeavesConifer | [0.035, 0.11, 0.05] | |
  | LeavesJungle | [0.04, 0.19, 0.03] | |
  | LeavesAcacia | [0.14, 0.20, 0.045] | |
  | Shrub | [0.22, 0.25, 0.10] | brightened from near-black (F-8) |
  | Regolith Highland / Maria / Ray | moon albedo tint | moon |
  | RegolithSubsurface | tint × (0.72, 0.70, 0.68) | moon |
  | LunarRock | tint × (0.48, 0.47, 0.46) | moon |

- **Verdicts:** D-15 and D-1: "the current biome and block colors are satisfactory and can be tweaked later when textures are added". The 2026-07-09 verdict: final tint = each texture's average colour.
- **Port:** as-is, as M1 block definitions with stable ids.
- **Milestone:** M1; textures M3.

### 3.2 Surface block selection
- **Looks like:** snow beats cliffs, cliffs bare stone or rock, shores get sand, and otherwise the biome's block.
- **Where:** `voxel.rs:866-924` (`surface_mat`); steepness at `2237-2243`.
- **How (priority order):**
  1. Underwater floor: gravel if more than 4 blocks deep, else sand.
  2. Lake shoal: sand.
  3. Lake-shore dither: sand.
  4. River bank: sand or gravel (§8.10).
  5. Climate Snow: Snow, even on cliffs.
  6. `steep ≥ 5` blocks: Rock.
  7. `steep ≥ 3`: Stone.
  8. Coastal beach: Sand.
  9. Otherwise the climate main block.

  `steep` is the max \|Δground0\| to the 4 neighbours, measured on **natural** ground so player towers are not cliffs. The far mesh greyed by slope instead (`terrain.rs:2851-2858`). That disagreement was D-10 / V-9.
- **Verdicts:** D-10 ("renderers may disagree if it cancels at scale"), moot with one renderer. Andrew 2026-07-11: shoals wear sand.
- **Port:** as-is; the block thresholds are scale-free. Expect more Stone and Rock because slopes are steeper. Drop the mesh twin.
- **Milestone:** M1; the lake and river parts M2.

### 3.3 Köppen class to main block
- **Where:** `planet.rs:216-231`.
- **How:**
  - BWh (3), BWk (4) and 255 → Sand.
  - EF (29) → Snow.
  - Everything else, steppe included → Grass.
  - Rock and Stone come only from slope.
- **Port:** as-is. **Milestone:** M1.

### 3.4 Climate tint palette
- **Looks like:** grass colour follows continuous temperature and precipitation. Dry or cold land goes olive-grey, the wet tropics deep green, steppe gold-green. Köppen class only nudges hue, and same-block transitions are kilometres long.
- **Where:** `planet.rs:2997-3024` (`ground_tint` anchors), `3052-3118` (ramps), `3308-3323`.
- **How:**
  - **Grass:**
    - `dry_limit = 100 + 35·max(T, 0)` mm; `moisture = smoothstep(dry_limit, dry_limit + 900, P)`.
    - Temperature anchors at T = −4 / 4 / 16 / 26 °C, smoothstep between them:
      - dry ramp [0.220, 0.215, 0.150], [0.260, 0.240, 0.110], [0.180, 0.230, 0.070], [0.205, 0.240, 0.055];
      - wet ramp [0.070, 0.160, 0.070], [0.075, 0.205, 0.060], [0.090, 0.250, 0.050], [0.050, 0.230, 0.040].
    - Final `mix(dry, wet, moisture)`, then 14% toward the bilinear Köppen anchor, rescaled to the base luminance.
  - **Sand:** `mix([0.545, 0.470, 0.290], [0.585, 0.485, 0.255], smoothstep(−4, 26, T))`, then 10% toward [0.485, 0.430, 0.270] by `smoothstep(250, 1400, P)`.
  - **Snow:** `mix([0.795, 0.835, 0.900], [0.835, 0.865, 0.905], smoothstep(−30, −4, T))`, then 12% toward [0.850, 0.875, 0.915] by `smoothstep(50, 900, P)`.
  - **Köppen anchors:**

    | Class | Anchor |
    |---|---|
    | Af/Am | [0.050, 0.230, 0.040] |
    | Aw | [0.200, 0.240, 0.055] |
    | BWh | [0.480, 0.360, 0.190] |
    | BWk | [0.380, 0.310, 0.210] |
    | BSh | [0.300, 0.260, 0.095] |
    | BSk | [0.260, 0.240, 0.110] |
    | Cs | [0.180, 0.230, 0.070] |
    | Cw | [0.090, 0.240, 0.055] |
    | Cf | [0.085, 0.250, 0.048] |
    | Ds | [0.150, 0.210, 0.085] |
    | Dw | [0.095, 0.210, 0.060] |
    | Dfa/b | [0.085, 0.220, 0.052] |
    | Dfc/d | [0.055, 0.150, 0.062] |
    | ET | [0.220, 0.215, 0.150] |
    | EF | [0.780, 0.820, 0.880] |
    | 255 on land | sand fallback [0.52, 0.45, 0.27] |

    The 255 fallback used to be navy and painted blue plates on desert coasts, which is Andrew's "blue tiles" note.
- **Verdicts:** 2026-07-09: long gradients within a main block, short ecotones across blocks. D-1: "colors need a little fine tuning". D-15: provisional until textures.
- **Port:** as-is, tinted per column from the bake's climate.
- **Milestone:** M1; the palette is owned by M3.

### 3.5 Biome domain warp
- **Looks like:** biome borders that do not follow the raster's texel shapes.
- **Where:** `planet.rs:65-85` (knobs, "Andrew's continental biome-warp art-direction knobs"), `386-481`, `2376-2425`.
- **How:**
  - 5 octaves of value noise in three rotated domains: base wavelength 5 texels, amplitude 0.65 texels, lacunarity 3.6, persistence 0.36, capped at 1.2 texels.
  - On Neisor that was about 85/24/6.5/1.8/0.5 km wavelengths with 11/4/1.4/0.5/0.18 km amplitudes.
  - Scaled by the gnomonic metric; seed `(seed + 0x0B100A2E)·7919 + octave·131`.
  - A warp that pulls a sea sample onto land is rejected.
  - Only class, temperature and precipitation warp; terrain, water and weather never do.
  - Applied only near class edges (the `climate_edge` fast path).
- **Verdicts:** D-3 "Leave as-is."
- **Port:** keep it, with parameters tied to the new bake's texel size. That makes it a few-hundred-metre warp **(inferred)**. Drop the gnomonic factor on the equal-area grid.
- **Milestone:** M1.

### 3.6 Cross-block ecotone dither (Grass, Sand, Snow)
- **Looks like:** desert/grass borders form fractal salt-and-pepper zones, with islands of each reaching into the other.
- **Where:** `planet.rs:87-154`, `535-564`, `2536-2640`, `2760-2786`, `3198-3218`.
- **How:**
  - Block weights ramp over `BIOME_BOUNDARY_ZONE_KM = 8.0` (clamped 0.3 km to 0.9 texel), with a 4×4 prefilter (side weight 0.15).
  - The comparator is 10 octaves of gradient noise in the warp's domains (about 85 km down to 0.8 m).
    - Amplitudes 1, 0.55, 1, 0.5, 0.25 …; octave 2 is a deliberate reset.
    - Normalised, ×(−1)·2.70, through a normal CDF, giving a uniform comparator.
  - The first block whose cumulative weight exceeds the comparator wins. Evaluated per column, so every block is crisp.
- **Verdicts:** D-3 "Leave as-is". D-18: irregularity is its own dial, and Andrew owns it.
- **Port:** keep the per-column fractal. Restate octaves in metres: about 6 octaves, with the 8 km zone becoming about 84 m **(inferred)**. **Ask Andrew** whether "as-is" means proportional or absolute widths. Coarse LOD voxels must pick by coverage of the same field (B-19).
- **Milestone:** M1.

### 3.7 Mid-scale ground patchiness (L-1(c))
- **Looks like:** warm-bright and cool-dark patches at 1.4 km, 350 m and 88 m that keep flat land from reading as one flat colour at medium altitude.
- **Where:** `shader.wgsl:247-256` (knobs "for Andrew"), `662-707`, applied at `1429-1431`.
- **How:**
  - 3 world-anchored `kgnoise` octaves at f = 4500, frequency ×4 and amplitude ×0.62 per octave; each octave retires at `1 − smoothstep(0.35, 0.9, cells/px)`.
  - `swing = clamp(0.11·value, ±0.16)`; multiplier `(1 + 1.18s, 1 + s, 1 + 0.72s)`, clamped to 0.8–1.2.
  - Water excluded, snow at 0.35 strength, off beyond 220 km.
- **Verdicts:** Austin approved it from A/B frames (BUGS L-1).
- **Port:** rethink. Block textures give no kilometre-scale variation, so keep this as a block-level tone field (dry patches, soil tone) or as a far-LOD tint, restated in metres.
- **Milestone:** M3 (look panel) or M5 (far view).

### 3.8 Per-block brightness mottling
- **Looks like:** the subtle ±10% block-to-block checker that reads as "voxel".
- **Where:** `voxel.rs:2265-2270`: `0.9 + 0.2·(hash(face, ci, cj, z) >> 17 & 0xFF)/255`. Trees use `0.88 + 0.24·…` (`2781`). The mesh twin was `shader.wgsl:1459-1474`.
- **Verdicts:** D-2: "Blocks will eventually have textures for variation so the mottling will eventually be removed."
- **Port:** a placeholder until textures. Give it a distance fade so coarse LODs don't shimmer. Drop the mesh twin.
- **Milestone:** M1 placeholder; removed in M3.

---

## 4. Geological strata (Austin named this)

### 4.0 What existed
There were exactly two mechanisms, and **neither was tied to planetgen's geology**. Every deep block was the same `Stone`, and the coloured beds existed only as a fragment-shader multiplier.

### 4.1 The soil column (voxel layering)
- **Looks like:** dig or look at a step and you see grass on top, then dirt, then stone. Desert is sand, then stone.
- **Where:** `voxel.rs:973-985` (`sub_mat`), `987-1016` (`mat_at`); side faces emitted as per-material runs at `2333-2423`.
- **How:** with `d = ground0 − z`, always measured from the **natural** pre-edit ground:
  - d ≤ 0 → surface block;
  - 1 ≤ d ≤ 3 → `sub_mat(surface)` (Grass/Snow → Dirt; Sand → Sand; Gravel, Rock and Stone → themselves);
  - d ≥ 4 → Stone.
  - Player blocks above ground0 are surface on top and substrate inside, so a tower on grass is a grass-capped dirt tower.
  - Moon: 4 blocks of RegolithSubsurface, then LunarRock (`voxel.rs:1000-1009`).
  - The depths are the same on every biome. The module doc says "sand over sandstone", but **no sandstone material exists**.
- **Verdicts:** bug R5-4 (a dug shaft floor rendered as grass) is why depth is measured from ground0.
- **Port:** rethink.
  - Keep "measure from natural ground".
  - Replace the fixed 3 blocks with a soil depth from planetgen's `soil_km` (0–0.2 km: thick on warm, wet, flat ground, thin on steep rock; `hydrology.py:413-417`). At 1/70 that is 0–3 m, about 1–4 blocks.
  - Add real sandstone under deserts.
  - Put the bed palette (§4.4) under the soil instead of uniform Stone.
- **Milestone:** M1. "Strata" is in the generator v1 list.

### 4.2 Shader bedding on cliffs ("Track C", shipped 2026-07-16)
- **Looks like:** faint horizontal colour beds about 9 m thick on steep exposed rock, earth and sand: canyon walls and mountain faces. Deliberately subtle, about 8% at most per channel. Flat ground, grass, snow, ice, trees and the moon are never banded.
- **Where:**
  - Knobs `shader.wgsl:233-246`; palette `257-263`.
  - `khash`/`kgnoise` `534-569`: bit-exact u32 twins of `noise.rs`, seeds premultiplied on the CPU.
  - Family classifier `571-591`; `strata_multiplier` `593-660`.
  - Call site in `fs_main` `1419-1428`, after biome reconstruction and before patchiness, weather and lighting. One function serves mesh tiles and voxel chunks.
  - Write-up `docs/archive/reviews/sol-strata-findings-2026-07-16.md`; evidence in `viewer/interchange/strata/` (untracked, §0.5).
- **How (re-implementable):**
  1. Return 1.0 if the fragment is 80 km or more away, or if the cheap reject passes: `1 − |n·camera_up| ≤ 0.075` (about 22°).
  2. **Material family**, from the original albedo by ratio tests:
     - *neutral* (rock, stone, gravel): chroma `(hi − lo)/hi` below `smoothstep(0.10, 0.22)`, and `hi` inside `smoothstep(0.12, 0.18)` to `1 − smoothstep(0.43, 0.52)`;
     - *earth* (dirt, sand): `(r − g)/hi` in `smoothstep(0.07, 0.14)`, `(g − b)/hi` in `smoothstep(0.10, 0.22)`, and `hi` in 0.27–0.34 up to 0.70–0.78.
     - Grass, leaves, logs, snow and ice fail **(inferred from the ratios)**.
  3. **Exposure:** `exposed = smoothstep(0.08, 0.28, 1 − |n·surface_dir|) · family · (1 − smoothstep(25, 80, dist_km))`. Beds begin at about 23° and are full by about 44°.
  4. **Elevation**, rebuilt camera-relatively so beds don't crawl: `elev_m = 1000·(cam_h + radial + (|rel|² − radial²)/(2·eye_radius))`.
  5. **Fold:** `folded = elev_m + 260 · kgnoise(surface_dir · 96, seed + 85021)`. That is one octave on a lattice of about 90 km (the comment says "about 175 km wavelength").
  6. **Bed:** `bed = folded / (9 m · thickness_scale)`; colour `PALETTE[khash(floor(bed), 17, −31, seed) % 5]`.
     - Hard but anti-aliased bed edges: cross-fade only in the top `aa = clamp(fwidth(bed)·0.5, 0.015, 0.35)`.
     - Beds retire when a pixel spans one: `resolved = 1 − smoothstep(0.55, 1.15, fwidth(bed))`.
  7. **Result:** `mix(1, band_colour, exposed · resolved · STRATA_BAND_STRENGTH)`.
- **Knobs:**

  | Knob | Value |
  |---|---|
  | `STRATA_BAND_STRENGTH` | 0.28 |
  | `STRATA_BAND_THICKNESS_SCALE` | 1.0 |
  | `STRATA_REFERENCE_THICKNESS_M` | 9.0 |
  | `STRATA_TILT_AMPLITUDE_M` | 260.0 |
  | `STRATA_FOLD_FREQUENCY` | 96.0 |
  | `STRATA_SEED_DELTA` | 0xDED7DCEC (the seed + 85021 stream) |

- **Palette** (colour multipliers):

  | Bed | Multiplier |
  |---|---|
  | cool shale | (0.72, 0.80, 0.88) |
  | pale ochre | (1.18, 1.09, 0.91) |
  | umber | (0.82, 0.77, 0.72) |
  | warm sandstone | (1.10, 0.98, 0.82) |
  | pale mineral bed | (0.91, 0.97, 1.05) |

  At full strength on Stone [0.30, 0.29, 0.28] they give (0.216, 0.232, 0.246), (0.354, 0.316, 0.255), (0.246, 0.223, 0.202), (0.330, 0.284, 0.230) and (0.273, 0.281, 0.294) **(arithmetic)**.
- **Character:** a 260 m fold over about 90 km is a dip of about 0.3%. The beds read as flat, Grand-Canyon horizontal layers; the fold mostly changes which bed sits at which height from region to region **(inferred)**.
  - On blocks, bed boundaries cut through the middle of block faces.
  - Shallow steps with partly smoothed riser normals (§10.5) stay unbanded; real cliffs band.
- **Cost:** +0.05 ms on the ground and +0.23 ms at 250 km orbit.
- **Verdicts:** none from Andrew in DECISIONS.md. L-1(c) named strata as a cure for flat medium-altitude colour. Austin, 2026-10-24: retain it. DESIGN.md: "geological strata (now as real blocks)".
- **Port:** drop it as a shader effect and make it real blocks (§4.4). Hand the palette, the bed thickness and fold numbers, and the "hard edges, subtle palette" look to the generator. The family classifier, distance fade and `fwidth` retirement all go away.
- **Milestone:** M1.

### 4.3 Lithology planetgen has but nobody uses
- **Where:** `planetgen/tectonics.py:412-416`.
- **How:**
  - `hardness = 0.8 + 0.45·fbm(f3) + 0.25·fbm(f12) + 0.3·taper·craton`.
  - `erodibility = clip(1/hardness, 0.4, 2.5)`.
  - `volcanic` = active arcs (> 0.4) or \|rift\| > 0.4.
  - Also `craton`, `plate`, `seafloor_age`, `uplift`, `subsidence`.
  - Inside planetgen, erodibility drives erosion and volcanic drives fertility. None of these reach the viewer, and hardness, volcanic and uplift are not even in `planet_data.npz`.
- **Port:** export them in the M1-bake.
- **Milestone:** M1, if the beds should follow geology (§4.4).

### 4.4 Recommendation: strata as real blocks (proposal, not yet decided)
- **Beds:** `bed = floor((z_world + fold(x, y)) / thickness_b)`, with a hashed or lithology-weighted pick from a small stone palette. Start from the five above **at full strength**: shale, ochre mudstone, umber, sandstone, pale limestone.
- **Rescale; don't port the numbers.** 9 m ÷ 70 = 13 cm, which 1 m blocks cannot show. Suggested starting values for Andrew to tune:
  - beds 2–6 blocks thick, with thickness hashed per bed;
  - folds of 5–30 m amplitude on a 1–3 km wavelength, so dips are visible at the new scale.
- **What you get free:** cliffs, canyon walls, cave walls and mine shafts all show bedding; mining becomes stratified; LOD voxels downsample real block colours.
- **Optional geology tie-ins** (need §4.3 in the bake):
  - volcanic → basalt beds;
  - craton interiors → a granite or metamorphic basement under thin beds;
  - erodibility → hard caprock against soft slope-formers (benches, and maybe mesas, §2.4);
  - a limestone bed in a wet climate → karst caves (§7.1, D-11).
- **Milestone:** M1 for the beds and soil column; M3 for Andrew's stone block art.

---

## 5. Trees and vegetation

### 5.1 Tree placement lottery
- **Looks like:** naturally clumpy forests, never rows.
- **Where:** `voxel.rs:1155-1218` (`tree_hash01`, `tree_at_scaled`, `tree_at`); hash `planet.rs:321-335`.
- **How:**
  - `hash_u64(face, ci, cj, salt)`: `x = (ci ^ (face << 60))·0x9E3779B97F4A7C15 + cj·0x85EBCA77C2B2AE63 + salt·0xC2B2AE3D27D4EB4F`, then the splitmix finaliser (`x ^= x>>30; x *= 0xBF58476D1CE4E5B9; x ^= x>>27; x *= 0x94D049BB133111EB; x ^= x>>31`).
  - `hash01 = ((h >> 11) & 0xFFFFFFFF) / 2³²`.
  - A tree grows iff `hash01(face, ci, cj, seed ^ 0x7255) < density`. Density is per column, roughly per m².
- **History:** an early hash that was linear in (ci, cj) made "orchard rows". Keep the avalanche.
- **Port:** as-is on the new (face, i, j). Two shape salts (`0x9E11` height, `0xF0F0` raggedness) are **unseeded**; include the seed this time.
- **Milestone:** M1, as a generator v1 addendum. It is cheap, carries biome identity, and the chunk-margin design (§5.6) must be settled before chunk generation hardens.

### 5.2 Species and density by biome
- **Where:** `voxel.rs:1028-1151`; `koppen_forest` `planet.rs:3026-3036`.
- **How:**
  - **Class table:**

    | Köppen class | Species | Density |
    |---|---|---|
    | Af, Am | Jungle | 0.011 |
    | Aw | Acacia | 0.0012 |
    | BSh | Acacia | 0.0005 |
    | BSk | Shrub | 0.0015 |
    | Cs | Broadleaf | 0.0025 |
    | Cw, Cf, Dwa/b, Dfa/b | Broadleaf | 0.005 |
    | Ds | Conifer | 0.003 |
    | Dwc/d, Dfc/d | Conifer | 0.007 |
    | ET | Shrub | 0.002 |
    | deserts, EF, ocean | none | — |

  - **Forest signature** (bilinear across texels): Af/Am 0.85, taiga 0.6, temperate 0.5, Ds 0.4, Cs 0.3, Aw 0.15.
    - Piecewise-linear density knots: (0, 0), (0.15, 0.0012), (0.30, 0.0025), (0.40, 0.003), (0.50, 0.005), (0.60, 0.007), (0.85, 0.011). Groves thin gradually across biome edges.
  - **Climate rules:**
    - Trees grow only on Grass.
    - Off-class species: T < 5 °C → Conifer; T > 20 and P > 1400 → Jungle; T > 16 and P < 1400 → Acacia; else Broadleaf.
    - Climate is read at the warped, dithered biome position, the same field as the ground colour.
  - **Canopy cover:** the comment says 0.010 is "closed canopy". At about 17–21 leaf cells per crown, 0.011 is only about 20% plan cover **(inferred)**. Check with Andrew if he wants a closed jungle roof.
- **Verdicts:** 2026-07-09 (trees blend across ecotones by the same hash). Andrew's note 3 (2026-07-11): "vegetation stopped at razor lines … trees spawned on beaches". Fixed by the shared warped field; at 27.903 33.965, trees on sand went from 23 to 0.
- **Port:** as-is. Per-m² densities survive the shrink; biome regions get 95× smaller.
- **Milestone:** M1.

### 5.3 Tree exclusions
- **Where:** `voxel.rs:1183-1207`, `1286-1310` (`tree_here`, mirrored in the mesher at `2720-2742`).
- **How:** no tree on:
  - water, the lake band (dry ground 0–1.5 m above a lake level), or carved river/pond ground (except river islands);
  - edited columns (`ground != ground0`), cave mouths (`top_solid != ground`), coastal beach sand, or river-bank material;
  - ground colder than `TREE_MIN_TEMP_C = −6 °C` annual (shrubs to −11 °C);
  - **relief**: \|Δground0\| > 2 blocks at the 8 probes (±2, 0), (0, ±2), (±1, ±1), or any carved non-island probe.
- **History:** this fixed buried crowns on slopes, leaf scraps over gullies, and "tower wants to become a tree".
- **Port:** keep the relief rule (block-scale). Beach and lake bands must follow the new block-scale shore rules (§8.5). In 3D, "cave mouth" becomes "open sky and solid ground under the anchor".
- **Milestone:** M1.

### 5.4 Tree shapes and colours
- **Where:** `voxel.rs:1161-1281`.
- **How:** `h_var = floor(hash01(…, 0x9E11) · 3)` gives 0–2.

  | Species | Trunk (blocks) | Total height | Canopy |
  |---|---|---|---|
  | Conifer | 4–6 | +2 | Stacked diamonds: layer z = 2 + l with Manhattan radius `clamp((trunk − l)/2, 0, 2)`, plus 2 tip leaves |
  | Broadleaf | 3–5 | +2 | Three layers at trunk−1, trunk, trunk+1 with square radius 2, 2, 1. A cell is dropped when \|dx\| + \|dy\| > r + 1 − bit, with bit = `(rnd >> ((dx+2)·5 + dy + 2)) & 1`: corners always go, other edge cells go at random. The same bits apply in every layer, so raggedness is vertical. A cap block sits on top |
  | Jungle | 6–8 | +2 | Same as Broadleaf |
  | Acacia | 3–4 | +1 | One flat 21-cell umbrella (\|dx\| + \|dy\| ≤ 3) |
  | Shrub | 0 | 1–3 blocks | No log |

  The log is written last and wins overlaps. Colours are in §3.1 and are fixed, not climate-tinted.
- **Verdicts:** D-15 and D-1 (colours satisfactory). LANDSCAPE Pass 3 (more species and silhouettes; Andrew specs them). `development-direction.txt`: roots and branches embedded in leaves and ground.
- **Port:** as-is. The shapes are block-scale.
- **Milestone:** M1; new species and branch blocks in M3.

### 5.5 Tree physics and interaction
- **Where:** `voxel.rs:1798-1893` (trunks solid), `1938-1964` (picking marched only the column surface).
- **How:**
  - Non-shrub trunks are solid; leaves and shrubs are passable.
  - You could not break a leaf or log: editing the anchor column deleted the whole tree.
- **Port:** rethink. Trees become ordinary Log and Leaf blocks in chunks, breakable one at a time. **Andrew decides** whether leaves are solid. Falling trees (DESIGN "things that move" #2) need a flood-fill over connected log and leaf blocks **(inferred)**.
- **Milestone:** M1b (breaking, trunk collision); M4 (falling trees).

### 5.6 Cross-chunk and cross-face determinism
- **Where:** `voxel.rs:42-45` (`TREE_MARGIN = 4` = canopy reach 2 + relief probe 2), `2685-2749`, `1967-2000`.
- **How:** chunks gather anchors within the margin. Anchors hash canonical (face, ci, cj), which fixed trees decided differently on two sides of a face edge. Edits remesh the ±4 neighbourhood across faces.
- **Port:** keep, and add **vertical** margins for 32³ chunks: a jungle crown reaches about 10 blocks, a mega erratic 4.
- **Milestone:** M1.

### 5.7 Seasonal foliage tint
- **Where:** `weather.rs:583-612` (`deciduous_tint`, strength 0.22 at line 193); applied at `voxel.rs:2784-2793`; annual-only positions at `voxel.rs:655-657`, `826-835`.
- **How:**
  - Only LeavesBroad changes.
  - `autumn = smooth(14→4, T) · smooth(−2→8, T) · smooth(0→8, −trend)`, toward [0.34, 0.16, 0.025].
  - `dormant = smooth(4→−6, T)`, toward [0.12, 0.115, 0.055].
  - Amount = 0.22·max(autumn, dormant).
  - Positions, species and density use **annual** climate, so "trees do not teleport". There is no snow on canopies.
- **Port:** as-is. **Milestone:** M5 (seasons).

### 5.8 Forest impostors (billboards): drop
- **Looks like:** past the voxel patch, each tree became two crossed tapered quads with a species silhouette, out to about 34 km, then a darker tint.
- **Where:** `terrain.rs:1632-1694`, `2006-2224`; shader `201-219`, `889-980`, `1189-1240`.
- **How (keep the method if any stand-in survives):**
  - Visible probability by distance: 1.0 to 1.8 km, 0.30 at 2.5, 0.16 at 4.3, 0.075 at 8.8, 0.010 at 18, 0 at 34 km.
  - Area-conserving scale `min(1/√p, 2.2)`, with height scaled by `scale^0.25`.
  - Density owned by world ticket and camera distance, **never by LOD level** (the B-18 fix).
  - Landing tiles dissolve in only the density delta.
  - Billboards were sized about 15% under the voxel footprint, because filled quads read larger; no bark, to avoid brown bands.
- **Verdicts:** 2026-07-09: "Forests: tree billboards at mid distance + vegetation coloring at high". Austin: "we fly at medium altitude a lot — this is important". The `TREE_IMPOSTOR_*` constants are labelled "Andrew's forest handoff knobs".
- **Port:** drop the billboards and **keep the intent**: groves with silhouettes at mid distance, tint at range. Do it with LOD downsampling that does not erase thin sparse features:
  - coverage-weighted leaf colour;
  - "any-leaf" occupancy at 2–4 m voxels;
  - canopy colour on the surface voxel at coarse LODs.

  This is untested **(inferred)**: get Andrew's look test before declaring the 07-09 verdict met.
- **Milestone:** design in M1 (LOD); any stand-in fallback in M5.

### 5.9 Far forest darkening
- **Where:** `terrain.rs:2832-2837`.
- **How:** `dark = 1 − 0.22 · forest · grass_weight`, using the same forest signature as placement.
- **Port:** should fall out of coverage-weighted LOD colour. Otherwise keep it as an explicit factor (0.22 is an art knob).
- **Milestone:** M1 with LOD colour; else M5.

### 5.10 Never built
Grass tufts, flowers, cactus, palms, birch, ferns, fallen logs, stumps (LANDSCAPE Pass 3), wind sway (WEATHER W5) and fireflies. Shrubs were the only ground cover.
- **Milestone:** M3 (non-full plant blocks); sway Later.

---

## 6. Rocks and boulders

### 6.1 Neisor rock lottery
- **Looks like:** isolated loose stones on ordinary ground; boulder fields on alpine, upland and desert ground; elongated outcrops; rare glacial erratics. Mountain slopes read as big-boulder country.
- **Where:** `voxel.rs:1318-1665` (`rock_tuning` 1318-1351, `neisor_rock_profile` 1380-1413, `rock_kind` 1438-1475, `rock_material` 1477-1504, `rock_cells` 1508-1613); write-up `docs/archive/reviews/sol-geology-findings-2026-07-16.md`; commits `1e2ca80` and `f86557c`.
- **How:**
  - **Family**, chosen in order:
    1. Alpine if Snow or elevation > 1.45 km;
    2. Upland if rough > 0.32 km;
    3. Coast if Sand and \|elev\| < 80 m;
    4. Arid if Sand;
    5. Forest if forest signature > 0.42;
    6. else Plains.
  - **Density** per column: Forest 0.00018, Plains 0.00025, Arid 0.00042, Coast 0.00032, Upland 0.00050, Alpine 0.00072.
    - ×(1 + 0.55·clamp(rough/0.36)), capped at 0.00112.
    - Ticket salt `seed ^ "ROCKLOTT"`. A denser trial "read as a carpet" and was rejected.
  - **Shape**, by thresholds on `q = hash(… "ROCKSHAP")`:

    | Ground | Single | Boulder | Outcrop | Erratic |
    |---|---|---|---|---|
    | ordinary | q < 0.60 | < 0.94 | < 0.995 | the rest |
    | exposed (upland) | < 0.43 | < 0.90 | < 0.992 | the rest |
    | alpine | < 0.43 | < 0.90 | < 0.975 | the rest |

  - **Templates:** fixed bottom-supported cell lists, rotated by a hashed quarter-turn.
    - Single: 1 cell.
    - Boulder: 3–6 cells, up to 2 high.
    - Outcrop: 7–11 cells, 5×3 footprint.
    - Erratic: 12–17 cells, 3 high.
    - Alpine MEGA_ERRATIC: 16–26 cells, a 3×3 base, 4 high.
  - **Material**, with `dark = hash < 0.46`:
    - Coast: Stone or Gravel.
    - Arid and Plains: Rock or Stone; every erratic is Rock.
    - Forest, Upland and Alpine: Rock or Stone.
  - **Placement:**
    - Anchors reject sea, water, the lake band, carved, edited and cave-mouth columns; there is no slope gate.
    - Each cell sits on its own column's ground, so outcrops follow the terrain; invalid cells are trimmed.
    - Trees win overlaps.
- **Verdicts:** LANDSCAPE Pass 1 says Andrew owns the palette per biome, the size distribution and erratic rarity. 2026-07-16 (Andrew and Austin): "Boulders collide; alpine erratics go mega".
- **Port:** as-is. Evaluate the family thresholds (1.45 km, 80 m, rough 0.32/0.36) on unscaled planetgen units (§0.2).
- **Milestone:** M1, in the same stamp pass as trees.

### 6.2 Rock collision
- **Where:** `voxel.rs:1737-1796` (`rock_solid_mask`, which mirrors the mesher's trims exactly; test at `3165`).
- **How:** boulders and larger are solid; singles were walk-through decoration, waiting for "a future non-cube pebble block".
- **Port:** free with real blocks. Singles become either a solid stone (a step hazard) or an M3 pebble block.
- **Milestone:** M1b (collision); M3 (pebble).

### 6.3 Rock stand-ins far away: drop
- **Where:** `terrain.rs:2246-2392`.
- **How:** two crossed tapered quads per rock.
- **Port:** drop; LOD voxels cover it.

*Lunar rocks are in §14.9.*

---

## 7. Caves and karst

### 7.1 Cave tubes and karst breaches
- **Looks like:** winding passages in a 26-block band that follows the terrain. Where a tube grazes the surface it opens as a pit or overhang: a karst breach.
- **Where:** `voxel.rs:41` (`CAVE_DEPTH = 26`), `577-609`; probe `viewer/examples/caveprobe.rs`.
- **How:**
  - Only on dry columns with `ground0 > 4` m.
  - Region gate: `gradient_noise(dir·90, seed + 40961) > −0.05`.
  - For k in 0..26 with `zm = ground0 − k`:
    - `n1 = gn(dir·(90000 + zm/12), seed + 31337)`;
    - if `|n1| < 0.085`, then `n2 = gn(dir·(76000 + zm/9), seed + 51413)`;
    - if `|n2| < 0.085`, carve.
  - Tubes are the intersection of two zero-shells, with height entering as a radial noise offset.
  - Lattice cells are 96 m × 12 m and 114 m × 9 m, so passages are probably flattish galleries 1–3 blocks tall **(inferred; confirm with caveprobe)**.
  - Anchored to natural ground; trees and rocks reject cave mouths.
- **Verdicts:** 2026-07-10, Austin: "karst frigging awesome", "No way are we going to want to suppress near-surface breaches"; breaches are canon. D-11 (Andrew): "I like the idea of cave generation being affected by biome properties in a somewhat-realistic way. I'll leave the specifics up to you" (wetter means more sinkholes).
- **Port:** rethink.
  - Restate frequencies in metres; `dir·90000` verbatim gives a 1 m lattice.
  - The "above 4 m" gate breaks at 1/70: most lowland is within 4 m of sea level **(inferred)**. Replace it with "some blocks above the local water table".
  - In real 3D, caves needn't live only in a 26-block band; consider deeper systems in the 512 m below sea level.
  - Couple density to climate (D-11) and to a limestone bed (§4.4).
- **Milestone:** M2, together with water tables and collapses. Dry caves could slip into M1 if cheap.

### 7.2 Flooded caves and karst pools
- **Looks like:** tubes below a nearby river, lake or sea level are water-filled. Breaches become blue pits and snake-shaped pools that read as ponds from the air.
- **Where:** `voxel.rs:611-646` (table), `348-353`, `2608-2676` (meshing), `1917-1930` (swim queries).
- **How:**
  - The table is the max of:
    - the river level, within `3hw + 50 m`;
    - the lake level, if `h < level + 10 m`;
    - 0 on coastal columns (`e_raw < 20 m`, ocean fraction > 0.5).
  - Used only if the table is ≤ ground0 and some carved cell lies below it.
  - Free top faces only under air; side faces only into dry passages, never against open sky.
  - Colour deepens toward [0.004, 0.013, 0.055].
  - Frozen columns render Ice and are walkable; the code supersedes BUGS W-6's "always liquid".
- **Verdicts:** Austin (W-6): "caves very near lakes and rivers will probably need to be full of water!"
- **Port:** rethink. These become generated water at rest (256 levels). Keep the rule "caves below the local groundwater table are water-filled, never perched", with the bands restated in blocks.
- **Milestone:** M2. M2's done-criterion ("digging into a lake drains it into your tunnel") exercises exactly this.

### 7.3 Difficulty Lake karst field (13.346, −4.807)
- **What it is:** lake 582. Its groundwater band floods dozens of near-surface tubes, giving a pond field. It is the named flagship site (DECISIONS 2026-07-10) and has world-reel poses.
- **Port:** rethink. The lake shrinks about 95× but galleries stay metre-scale, so the same coordinates may hold only a few breaches **(inferred)**. If the look is wanted, design a karst lake district on purpose: limestone, wet climate and a shallow table. Keep the coordinates as a test site.
- **Milestone:** M2.

### 7.4 Mesh karst hint: drop
- **Where:** `shader.wgsl:1475-1561`.
- **How:** a bit-exact WGSL twin of the cave field that painted pools on the far mesh.
- **Port:** drop it. Keep the principle: every generated structure is a pure function of (position, seed), so far LODs can show it.

*Cave darkness is in §10.4.*

---

## 8. Water: ocean, beaches, lakes, ponds, rivers, falls

**WATER-CONTRACT invariants worth keeping** (`docs/archive/WATER-CONTRACT.md`):
- **W1** shore at grade: resting water never stands above adjacent dry ground.
- **W2** never perched: water that would drain does not exist.
- **W3** structures that serve water (benches, fills, aprons) taper away with their water.
- **W4** one truth for generator, renderer and physics.
- **W5** no see-through water.
- **W6** open water featureless at rest; the ice checker is by design.
- **W7** wet/dry identical at every LOD.
- **W8** determinism.

W1–W3 are the generator-side twin of DESIGN's "generated water starts at rest".

### 8.1 The sea rule
- **Where:** `planet.rs:381-384` (`sea_from_fields`), `terrain.rs:555-565`.
- **How:** `sea = e_raw ≤ 0 && (water_mask ≥ 0.5 || (e_raw ≤ −100 m && ocean_frac > 0.35))`.
  - `water_mask` is the unblurred Köppen-255 mask.
  - The deep clause keeps straits and fjords wet.
  - Elevation alone produced "navy lagoons" and flooded Neisor's genuine dry below-sea-level basins.
- **Verdicts:** a viewer/CLAUDE.md invariant.
- **Port:** as-is. Evaluate the −100 m clause on unscaled height. Masks must agree across face seams (F-17).
- **Milestone:** M1.

### 8.2 Ocean colour and coastal bathymetry tint
- **Looks like:** teal shoals that darken to navy over depth, with a turquoise shelf.
- **Where:** `terrain.rs:2640-2661` (`water_surface_color`, the one shared ramp); shader decoder `shader.wgsl:265-271`, `709-723`, `1611-1622` (Track C).
- **How:**
  - `t = clamp(depth / 2.5 km)`; `c = mix([0.055, 0.17, 0.30], [0.004, 0.013, 0.055], t)`.
  - Sea only: `c = mix(c, [0.10, 0.32, 0.35], 0.7·clamp(1 − depth/20 m))`.
  - Salt lakes: `mix(c, [0.45, 0.55, 0.52], 0.55)`.
  - The shader adds `BATHYMETRY_SHALLOW_TINT` (0.035, 0.42, 0.36) at up to 0.55. It is decoded from vertex RGB, which is plumbing to drop.
- **Verdicts:** D-15 and D-1 (colours fine); TRANSITIONS F (one ramp, because copies drifted, V-1).
- **Port:** keep the colours as endpoints and re-key depth. A 2.5 km deep end becomes about 36 m, the 20 m shoal 0.3 m. Suggest shoals over 0–5 blocks and deep at 100–150 m.
- **Milestone:** M1; the knobs go to M3.

### 8.3 Gaps in water rendering
- Water was opaque in the terrain pass (`renderer.rs:1294-1300`), with no transparency, refraction, specular, Fresnel, waves or shoreline foam (grep confirmed).
- **Port:** new work. Translucent water with depth fog is a natural M3 look item. Keep W6.
- **Milestone:** M1 (opaque is fine); M3 (look).

### 8.4 Ocean floor and coastal floor
- Covered in §2.1: drop the −6 m clamp, keep coastal land at least 1 block above sea, and keep coastal roughness damping (pre-shrink).
- **Milestone:** M1.

### 8.5 Coastal beach sand
- **Looks like:** a sand band on low coastal ground with a fractal edge, calmer on calm coasts.
- **Where:** `terrain.rs:2419-2570`; `voxel.rs:926-937`.
- **How:**
  - `beach_frac = (1 − smoothstep(9, 13 m, e_raw)) · (1 − smoothstep(10, 16 m, h))`; sand if `comparator < beach_frac`.
  - Comparator: 4 octaves of gradient noise on the column centre, seed + 0x0BEAC4EC, base 4,096 columns, lacunarity 16 (about 7 km / 440 m / 27 m / 1.7 m). Octaves 1–3 are scaled by `0.08 + 0.92·smoothstep(25 m, 500 m, rough)`; then ×2.56 and a normal CDF.
  - The beach yields to lake territory.
- **Verdicts:** V-1 (one shared fraction); D-3 as-is.
- **Port:** rethink for 1/70. The 9–16 m band is 0.13–0.23 m. Either evaluate it on unscaled elevation or restate it as "0–2 blocks above sea plus distance to coast". Keep the roughness-adaptive fractal, in metres.
- **Milestone:** M1.

### 8.6 Lakes: creation and bake repair
- **Where:**
  - planetgen: `hydrology.py:42-66` (priority flood, eps 1e-6), `359-408` (lake extraction).
  - Bake: `scripts/bake_rivers.py:155-435`.
- **How:**
  - **planetgen:**
    - Depressions where `fill − elev > 1e-4` become lakes with spill = min(fill).
    - Open/fresh if inflow ≥ `pet_lake·area·3.171e-5`; otherwise salt, with the level set where evaporation equals inflow.
    - Lakes under 3 cells and under 30 m deep are skipped.
  - **Bake:**
    - Conduit peel: drop cells more than 300 m below spill, peel chain-like cells more than 250 m below; level = `min(rim, max bed + 5 m)` (W-5/F-12).
    - Renderability cap: level ≤ the outside minimum + 15 m.
    - A census-caps loop lowers lakes that make walls.
    - River nodes inside a lake are pinned to its surface; rim cells are exported with flag 2.
  - **Unit pitfall:** the lake `radius_km` is in planetgen (6,371 km) units, but `rivers.rs` measures on the 8,660 km sphere (`bake_rivers.py:421`) **(inferred mismatch)**. Use one unit system.
- **Verdicts:**
  - W-7 and W-5b: the honest fix is upstream ("don't merge depression chains").
  - D-14 (Andrew): "Small and/or considerably slanted terrace-like formations can exist but large cliffs shouldn't form around lakes in smooth-terrain areas."
- **Port:** rethink.
  - Fix chain-merging in planetgen.
  - The engine computes each lake's spill level on its own terrain; the census loop existed only because the bake couldn't see the viewer's relief.
  - Keep the peel as a fallback.
- **Milestone:** M2.

### 8.7 Lake footprint, beds, shelves and shore aprons
- **Where:** `rivers.rs:280-284` (`flood_eligible`), `1193-1344` (`lake_at`); `terrain.rs:689-836`, `1163-1180`.
- **How:**
  - **Flood test:** flood if `d < 2.6r && (inside the lake's Voronoi cell || (d < 1.15r && rim + 30 m ≥ level))`. That is, flood-through only across true dams.
  - **Bed flattening:** `env ×= 1 − 0.88·smoothstep(−5, 12 m, level − e)`.
  - **Inner shelf:** `level − 1.2 m − 3%·distance`.
  - **Outer apron:** where ground just outside would dip below the waterline, raise it to `level − 1 m − 3%·distance`. "The GROUND yields" instead of lowering levels; capping levels had killed 628 of 678 lakes.
- **Verdicts:** these enforce W1/W2; D-14's bar; bugs F-1 and W-1 are the two failures it threads between.
- **Port:** rethink. With a real spill solve on engine terrain, the Voronoi footprint only seeds which depression holds which lake. Keep:
  - flood-through only across real dams;
  - flattened lake beds;
  - the 3% grade (it survives the slope-preserving shrink);
  - "ground yields, gently".

  Lake depth becomes a design call: the median lake is about 0.34 m deep at 1/70. **Ask Andrew** about a minimum depth in blocks.
- **Milestone:** M2.

### 8.8 Lake shore sand, shoals, salt lakes, outlets
- **Shore sand (V-7):** `(1 − smoothstep(0, 1.5 m, h − level)) · (1 − smoothstep(100, 300 m, distance to edge))`, liquid lakes only (`terrain.rs:2572-2630`).
- **Shoals:** analog ground under the surface that lands in the same block as the water becomes a dry sandbar flush with the surface, darkened by ×(0.82, 0.82, 0.84) (`voxel.rs:885-893`, `2280-2286`). Andrew, 2026-07-11, quoted in code: "water-coloured walkable caps read as broken water … Shoals now wear SAND."
- **Salt lakes:** the planetgen flag gives the pale water colour (§8.2).
- **Outlets:** a descending outlet river owns its corridor when `wl < level − 8 m` (`terrain.rs:960-971`).
- **Port:**
  - Keep the 1.5 m band in blocks; restate the 100–300 m edge distance as about 1–4 blocks.
  - With 256 levels, an equal-block tie becomes a partial water cell, so the shoal hack drops. Keep the look rule: barely emergent ground at the waterline is dark wet sand.
  - Salt lakes as-is.
  - The outlet margin becomes block-scale.
- **Milestone:** M2.

### 8.9 Ponds
- **Looks like:** small flat pools in calm, wet, flat lowlands.
- **Where:** `terrain.rs:277-389`, `1003-1161`.
- **How:**
  - **Gates:** precipitation > 550 mm, `rough_r < 0.45`, `e_raw > 30 m`, envelope `env0 < 0.13`, coarse grade < 2% over about 0.5 km, outside lakes, aprons and river valleys.
  - **Mask:** `pn = fbm_band(dir, 0, 2, 16000, seed + 9241) < −0.50`.
  - **Shape:**
    - dig `(−0.50 − pn)·30 m`;
    - a **flat** level `e_raw − 1.8 m` (F-3);
    - a bench at `wl − 1.5 m`;
    - never perched;
    - a 3 m fill budget on coastal-mask fringes;
    - apron `wl − 1 m − (0.012x + 0.090x²)·env0·10`.
- **Verdicts:** F-3, F-22 (Austin's 18 m pond wall), A-4/A-5 ramparts fixed in `29188ec`. Dammed ponds are an open design item.
- **Port:** rethink. Anchoring the level to `e_raw − 1.8 m` was the root of the wall and rampart families. In real 3D, find a local depression in engine terrain and fill it to its own spill, so a pond can never need a dam. Keep the placement gates (rescale the 30 m one).
- **Milestone:** M2.

### 8.10 Rivers: courses, levels, width and depth, meanders, banks, bed
- **Where:** `bake_rivers.py:66-145`; `rivers.rs:25-146` (knobs, "Andrew-facing river art knobs"), `358-625`, `705-1176`; `terrain.rs:842-934`, `1182-1223`; `voxel.rs:536-575`, `941-962`.
- **How:**
  - **Courses:**
    - Segments with flow > 120 m³/s are exported.
    - Smoothed twice: `0.5·self + 0.25·main-upstream + 0.25·receiver`.
    - **Mouth nodes stay anchored**: smoothing had dragged a great river's mouth 40 km offshore and left it dry.
    - planetgen picks receivers with ±`river_meander` random weights and `1 + capture·log1p(acc/100)` valley capture, which makes dendritic networks.
  - **Levels:**
    - `level = max(elev, receiver level)` from terminals, with oceans pinned at 0.
    - Re-anchored after smoothing, then a min-relaxation downstream.
    - These are the **hydraulic fill surface, never bed-anchored** (F-4).
    - At query time, the level is the `(1 − d/3.5 km)²`-weighted mean of every segment within 3.5 km, so hairpins ramp instead of cliffing (W-2).
  - **Width and depth:**
    - `taper = smoothstep(120, 400, Q)`.
    - `half_width = 1.5 m·√Q·taper`.
    - `depth = min(0.27 m·Q^0.39, 12 m)·taper`.
    - ±0.34 width noise, floor 0.55.
  - **Meanders:**
    - Amplitude `280 m·(0.35 + 0.65·smoothstep(120, 800, Q)) + 0.10·hw`, capped at 620 m.
    - Wave `sin(2π(arc/4.2 km + φ + 0.75 + 0.18·noise)) · (0.82 + 0.18·|noise|)`: a warped sinusoid, so a large amplitude never reads as inert.
    - Reaches run junction to junction, pinned at confluences, sources, mouths and lake ends with a `smoothstep(0, 5 km)` envelope. Arc length is continuous, so there are no restarts per 30 km cell.
  - **Banks:**
    - `bank_w = max(hw·scale, 20 m) + 1.2·(h − wl)`: deep cuts widen into canyons.
    - Profile power `0.90 + 0.65·flowgain + 0.16·noise`; 16 m edge wobble at 0.95 km.
    - Asymmetry 0.18, keyed to noise × side.
  - **Bed and wetness:**
    - Parabolic bed `wl − 1.2 m − d_max·(1 − x²)`; water where `floor(wl) > floor(h)` (F-23).
    - Floodplain `valley` (§2.1).
    - Dips within `1.6·edge + 10 m` flood as free natural bays; beyond that an apron at `wl − 1 m − 3%·distance`.
    - **Dry washes:** `river_wet = 1 − smoothstep(2, 6 m, wl − h_ref)` turns perched graph water into a dry channel (W2).
  - **Bank material:**
    - Sand or gravel where `|h − wl| < 1.8–4.5 m` laterally near the edge, chosen per 64-column patch so the band never speckles.
    - Gravel probability `0.08 + 0.52·smooth(1200–18000 Q) + 0.24·cold + 0.10·wet + 0.22·rough`.
  - **Creek films:** creeks under 1.5 m half-width probe 3×3 sub-cell offsets and force a one-block film if any is wet (F-15).
- **Verdicts:**
  - D-12, Austin's pick: meander-curvature asymmetry (cut bank steep, point bar shallow) plus wet margins plus bank-width noise. Andrew agrees and adds: "the extent to which rivers meander should be a variable based on flow or width as it is in real life, e.g. the Lower Mississippi vs. the Upper Mississippi."
  - **Not implemented:** curvature-keyed asymmetry and wet margins **(read in code)**.
  - Andrew's note: "rivers can extend a few blocks over the terrain" (perched water, W2).
- **Port:** rethink for scale and D-12.
  - Keep the hydraulic laws (√Q width, Q^0.39 depth), monotone fill-surface levels, mouth anchoring, the reach-pinned warped-sinusoid meander, depth-dependent bank widening, bays, dry washes and climate-driven bank material.
  - At 1/95 horizontal, a 300 m river becomes 3 m and 0.1 m deep. Rescale in blocks with floors (at least 1 block deep). A softer width scale may be needed for great rivers to read as rivers; that is a dial for Andrew **(inferred)**.
  - Make meander wavelength and amplitude functions of channel width (about 10–14 widths), per D-12.
  - Key bank asymmetry to the analytic signed curvature of the meander.
  - With 256 levels, store `wl − h` fractionally: level blending, block-quantised flood tests and creek films collapse into shallow fractional cells.
- **Milestone:** M2.

### 8.11 Braided reaches and islands
- **Where:** `rivers.rs:87-107`, `627-691`; `terrain.rs:866-877`.
- **How:**
  - Braided if Q ≥ 2,200, gradient < 0.0009, and `hash < 0.42·flowgate·gradegate`.
  - Bars: one slot per 2.4 km with a 0.68 lottery; half-length 0.42–1.15 km; half-width 0.16–0.31 hw.
  - Bar ground `wl − 1.6 m + field·5.8 m`; the water level never changes (tested).
  - Trees may grow on river islands.
- **Port:** keep; rescale lengths to the new width.
- **Milestone:** M2.

### 8.12 Waterfalls, rapids and foam
- **Looks like:** prominent falls with a falling white sheet and a plunge, subtle centre-channel rapids, pale foam where water steps down.
- **Where:** `rivers.rs:109-146`, `422-515`, `901-1007`; foam `voxel.rs:2491-2605`; sheet `shader.wgsl:1725-1741`.
- **How:**
  - **Candidate strength:** `smoothstep(300, 1500, Q) · smoothstep(50, 180 m, drop) · smoothstep(0.6%, 1.4%, gradient)`, kept if ≥ 0.08.
  - **Prominence:** keep only the strongest within a 72 km arc, at least 96 km apart; losers become rapids (the B-13 fix).
  - **Profile:** `min(0.55·drop, 120 m)` concentrated in a ±2.5 m sheet with a 12% drawdown over 750 m before it; the endpoints stay bit-exact and monotone. The approved site is segment 8372 at 9.79546 107.60747.
  - **Rapids:** `0.18 · smoothstep(300, 1800, Q)` in the channel core; colour only.
  - **Step foam (F-14):** toward [0.70, 0.82, 0.78] at strength `0.10 + 0.06(d − 1)`, clamped 0.08–0.30.
  - **Fall foam:** toward [0.86, 0.94, 0.97].
  - **Sheet:** `streak = 0.5 + 0.5·sin(dot(pos, (83, 47, 109))·145 − 5.5t)`; whitewater `(0.30 + 0.42·streak + 0.22·plunge)`, capped 0.92, toward [0.94, 0.985, 1.0]. It runs on the deterministic clock.
- **Verdicts:** D-4 (Andrew): "Single-block increments are a result of rivers flowing downhill … will probably disappear … if water levels are added … In areas with rougher terrain, waterfalls with up to several dozen-block jumps could be implemented into generation and white particles could appear along the downwards trajectory and at the bottom." LANDSCAPE Pass 2 imagined spray mist and a sun-angle rainbow "trailer shot".
- **Port:** rethink. planetgen's 50–180 m drops become 1–3 blocks at 1/70, so the generator must build cliffs, steps and plunge pools where local relief allows, seeded by planetgen's fall candidates. Keep:
  - detection by flow × drop × gradient, with prominence and spacing;
  - the monotone endpoint-preserving profile;
  - the foam colours and the animated streak.

  Step foam at block increments goes away with 256 levels; key foam to slope and speed instead.
- **Milestone:** M2 (profile, foam); particles M5 or Later.

### 8.13 River currents
- None existed: the player was never pushed by flowing water. DESIGN says currents come from planetgen flow data.
- **Milestone:** M2.

*Frozen water is in §9.2. Underwater view and swimming are in §15.3. Census tools are in §19.*

---

## 9. Snow, ice and seasons

### 9.1 Snow ground and the snowline
- **Looks like:** cold land becomes Snow blocks with a per-column dithered edge. Köppen EF caps are white-blue.
- **Where:** `planet.rs:205-214`, `3100-3105` (`climate_snow`), `3268-3306` (`apply_snow_override`); comparator `518-533`.
- **How:**
  - Below −10.5 °C the ground is always Snow. Between −10.5 and −7.5 °C it is Snow if `T < −9 + (c − 0.5)·3`. Here `c` is a 4-octave value-noise comparator (seed + 0x5A0E, base 4,096 columns, lacunarity 16, normal CDF).
  - It uses the seasonal temperature when seasons are on; trees use annual.
  - Snow beats steepness; the substrate is Dirt.
  - **No sub-raster lapse rate exists**, so local peaks get no snow caps of their own. That is open question B-26.
- **Verdicts:** TRANSITIONS praises the dithered snowline ("it reads great"). D-15: colours provisional.
- **Port:** as-is (temperature-based, so scale-free), with one addition: a virtual lapse on unscaled height, `T − 6.5·(h_new·70.3 − h_raster)/1000`, so 120 m peaks get caps **(recommendation)**.
- **Milestone:** M1. The moving seasonal snowline is M5.

### 9.2 Frozen lakes, rivers and sea: walkable ice sheets
- **Where:** `terrain.rs:568` (annual < −4 °C); `planet.rs:1356-1446`; `weather.rs:614-630` (hysteresis); physics `voxel.rs:355-369`, `1716-1930`; visuals `voxel.rs:2456-2488`.
- **How:**
  - Annual mode: frozen below −4 °C annual mean.
  - The top is `Mat::Ice` mixed toward snow by `f²·0.6` (per-column hash) plus the brightness checker (W6: the ice checker is by design).
  - Solid to every query: stand, aim, place, torch, ceiling (F-18).
  - Shore aprons skip ice, which leaves ice cliffs (S-3).
  - Water quads skip snow dusting and rain darkening (V-9).
- **Verdicts:**
  - D-9 (Andrew): "Water should generate variably as either frozen or not based on the season according to temperature and weather statistics, and should update when revisited. Manually placed ice or water should only break this rule if temperature warming or cooling systems exist." The W4 bracket adds that player-placed water should *ideally* follow temperature too.
  - D-13: "both [ice cliffs and tapered glaciers] should definitely exist … the Antarctic coastline has large cliffs, while mountain valley glaciers tend to taper off."
- **Port:**
  - Real 3D improves this: ice becomes the top N layers over liquid, so you can dig through and swim under it.
  - Keep the annual rule for generator v1, plus the hysteresis, dither and edit precedence.
  - Ice-shelf cliffs versus tapers needs a generator rule.
- **Milestone:** annual polar sea ice in M1; lake ice with lakes in M2; seasonal behaviour in M5.

### 9.3 Structural seasons
- **Looks like:** lakes freeze in winter and thaw in spring; sea ice advances and retreats; the snowline moves; broadleaf trees go autumnal.
- **Where:** `weather.rs:505-630`, `1062-1157`; `planet.rs:3127-3196`; chunk refresh `renderer.rs:3441-3514` (at most 16 per frame).
- **How:**
  - 24 season buckets (4–96 allowed).
  - Seasonal temperature = annual + `a·cos + b·sin` (from `weather.bin`).
  - Inland water: freeze below −5 °C while cooling, thaw above −2 °C while warming, with a ±0.5 °C world-anchored dither so whole shores don't flip together.
  - Sea: monthly baked ice fraction against a comparator.
  - Grass stays annual.
- **Verdicts:** D-9; Austin W4.1: buckets step visibly under fast-forward ("make the increments smaller").
- **Port:** rethink. DESIGN's simulated water and saved changed chunks collide with "regenerate frozen state on revisit". Freezing should probably become a per-block, temperature-driven state change applied lazily on chunk load. Drop the canonical-bucket compatibility hack (`CANONICAL_SEASON_FRAC`).
- **Milestone:** tints in M5; structural freeze/thaw Later, after M2 water.

### 9.4 Glaciers
- Not built. There was no ice thickness or crevasses; EF was snow over dirt. LANDSCAPE banked "glacier tongues".
- **Port:** new work (D-13).
- **Milestone:** Later.

*Weather snow dusting is in §12.8.*

---

## 10. Lighting and the block look

### 10.1 Colour pipeline
- **Where:** `main.rs:330, 499, 2034` (sRGB surface); pass `renderer.rs:4309-4403`; clear colour (0.002, 0.003, 0.008).
- **How:**
  - Linear colour written straight to an sRGB target: **no tonemap, exposure, HDR or shadow maps**. Values above 1 clip (the sun is `tint·3.2`).
  - No MSAA.
  - Orbital frames quantise to 1/4096 for byte stability.
- **Port:** rethink. Start with an HDR target, a deterministic tonemap and dither, so Andrew's look panel has exposure and white point. Byte determinism must survive.
- **Milestone:** M1; dials in M3.

### 10.2 Terrain lighting equation
- **Where:** `shader.wgsl:1432-1458`, `1647-1673`.
- **How:**
  ```
  day      = smoothstep(-0.08, 0.15, dot(sun, up)) * (1 - eclipse)
  sky_hemi = clamp(0.5 + 0.5*dot(n, up), 0, 1)
  odim     = mix(1, overcast_sun_floor(0.35), cover*day*wcam)
  ambient  = (0.10 + 0.40*day*sky_hemi) * mix(1, 0.85, cover*day*wcam)
  sun_k    = mix(1.0, 0.60, day) * odim * day * (1 - 0.35*cloud_shadow*wcam)
  c        = albedo * (ambient + sun_k*max(dot(n, sun), 0))
  ```
  - Sun light is white; `sun_tint` is used only for the disc and halo.
  - `up` is the camera's up below 2.5 km, blending to per-pixel up by 40 km, and always per-pixel during eclipses.
  - A flat noon top gets about 1.10 × albedo, a side face about 0.74 of a top, an underside about 0.055 **(arithmetic)**.
- **Verdicts:**
  - The F-8 fix: a flat 10% ambient had made lee faces near-black.
  - D-7 (overcast gloom): "This can be adjustable, but I'd opt for whatever is realistic." The 0.35 floor was chosen for readable screenshots.
  - D-10: shading differences must cancel at scale. That now applies between voxel LODs.
- **Port:** keep the equation and constants as the M1 starting look, made per-pixel everywhere (§0.3). The coefficients (0.10, 0.40, 0.60, 0.35, 0.85) are the first look-panel sliders.
- **Milestone:** M1; overcast and cloud shadow M5; dials M3.

### 10.3 Night and moonlight
- **Where:** `shader.wgsl:1699-1724`.
- **How:**
  - `+= albedo·(0.40, 0.50, 0.72)·(0.10·max(n·moon, 0) + 0.015 + 0.05·hemi_n)·(1 − day)·smoothstep(0, 0.15, moon_alt)·phase·1.15`.
  - `phase = 0.5 + 0.5·dot(sun→moon, pixel→moon)`.
  - The `hemi_n` term ("sky-shaped night floor") fixed a low moon making terrace risers brighter than tops.
  - Direct sun × `day` fixed "tree shading is backwards" (canopies lit by a set sun).
  - **Gap:** moonlight was not dimmed by a lunar eclipse.
- **Port:** as-is, plus an eclipse factor.
- **Milestone:** M1c (24-minute days put nights in M1).

### 10.4 Cave darkness, the carry light, underwater tint
- **Where:** `voxel.rs:2288-2290`, `2363-2365`, `2629-2632`; shader `1682-1689`, `1817-1820`.
- **How:**
  - Faces dim by `clamp(1 − 0.20·(top_solid − z), 0.25, 1)` blocks below the column top. This is applied in the shader so light can lift it.
  - Carry light: `reach = clamp(1 − d/10 m)`; `c = c·dim + albedo·(1.0, 0.78, 0.48)·reach²·(1 − dim)·0.85`. It acts only where dark, so daylight pixels are unchanged.
  - Underwater: `mix(c, (0.02, 0.07, 0.16), 0.6)`.
- **Port:** rethink. "Depth below the column top" only works for heightfield-plus-cave-band columns. Player tunnels in real 3D need real sky visibility: skylight propagation, Minecraft-style, or shadows. Otherwise tunnel interiors are sunlit. Keep the warm carry light as a cheap stand-in.
- **Milestone:** M1c (M1 is "dig a tunnel").

### 10.5 Per-face block shading (the seven-fix "shading saga")
- **Where:** `voxel.rs:2185-2423`, `2559`, `2806-2836`.
- **How:**
  1. **Top normals** come from central differences of the **continuous** height, not integer block tops. Integer differences tilted one ring of columns about 27° at every terrace edge (F-2, F-5). Edited or carved neighbourhoods fall back to integer differences.
  2. **Side-face directions** are each column's own grid axes. Position-derived normals made the pattern repeat on the 32-column chunk grid.
  3. **Side normal** `normalize(out + 0.85·up)` (about 40° toward the sky). It blends toward the top normal near the surface: `k = clamp(0.85 − 0.28·depth, 0, 0.85)`. Player walls keep the cube normal.
  4. **Baked multipliers:**
     - risers ×0.90 (it was 0.72, which smeared; F-11);
     - undersides ×0.55;
     - trees: sides ×0.8, bottoms ×0.6;
     - water sides ×0.93, with normal `normalize(out·0.18 + up)` (F-9: steps had alternated bright and black).
- **Port:** items 2 and 4 as-is. Items 1 and 3: use the generator's continuous surface normal for natural tops at **every** LOD, so 1 m and 16 m voxels shade alike (D-10) **(inferred recommendation)**. Use cube normals for edits and overhangs.
- **Milestone:** M1.

### 10.6 Ambient occlusion
- **Where:** `voxel.rs:2293-2305` (tops), `2392-2416` (walls).
- **How:**
  - Per corner: `lvl = (s1 && s2) ? 3 : s1 + s2 + diag`; brightness `1 − 0.15·lvl` (1.0, 0.85, 0.70 or 0.55).
  - Wall AO was added after Austin saw the shadow stop dead at the crease.
  - The quad diagonal is fixed, with no AO-aware flip.
  - A 4-column ghost margin makes AO agree across chunks.
- **Port:** as-is for 1 m blocks. Add the diagonal flip. Scale AO strength or footprint by LOD so coarse voxels don't darken 2^k-metre spans.
- **Milestone:** M1.

### 10.7 World-stable shader noise
- **Where:** `shader.wgsl:534-569` (`khash`/`kgnoise`), `1838-1871` (`hash3i`, `vnoise_at`); camera-anchored integer lattice at 25 m cells `renderer.rs:4015-4033`.
- **How:** integer hashes on exact lattices, offset by a camera anchor. Never `fract()` hashes or raw planet-magnitude f32. F-25: raw positions quantised at 0.24 m and crawled; `hash31` made axis shards; a cell-constant fix read as a crawling 25 m checker.
- **Port:** as-is, for every world-anchored shader pattern.
- **Milestone:** M1.

### 10.8 Cube-sphere frames
- **Where:** `planet.rs:15-22` (`FACES`), `338-374`; `voxel.rs:442-459`.
- **How:**

  | Face | axis | right | up |
  |---|---|---|---|
  | 0 | +X | +Y | +Z |
  | 1 | −X | −Y | +Z |
  | 2 | +Y | −X | +Z |
  | 3 | −Y | +X | +Z |
  | 4 | +Z | +Y | −X |
  | 5 | −Z | +Y | +X |

  - `face_dir = normalize(axis + u·right + v·up)`.
  - `face_from_dir` picks the largest dot with a strict `>`, so ties go to the lowest index.
  - Column centre `−1 + 2(i + 0.5)/N`.
- **Verdicts:** DESIGN: frames match. Andrew: the 8 cube corners are "part of the charm".
- **Port:** as-is. `crates/cubesphere` already matches. Prefer exact integer cross-face neighbours over the old float round-trip.
- **Milestone:** M1 (M1-grid).

### 10.9 Reversed-Z, infinite far plane, f64 camera
- **Where:** `camera.rs:1-56`, `105-131`, `224-236`; `renderer.rs:4083-4097`; sky unprojection `shader.wgsl:2411-2418`.
- **How:**
  - `perspective_infinite_reverse_rh(65°)` with near = `clamp(alt·0.08, 0.1 m, 50 km)` (about 14 cm at eye height, inside the 35 cm body radius).
  - Depth32Float with a `Greater` compare, cleared to 0.
  - The sky unprojects the **near** plane; unprojecting the far plane blacked out the sky below about 8 m.
  - f64 km positions; GPU data is camera-relative f32.
  - Beyond 50,000 km one rigid shared anchor stops far bodies jittering tile by tile (Andrew: "celestial bodies jitter violently").
- **Verdicts:** a viewer/CLAUDE.md invariant.
- **Port:** as-is.
- **Milestone:** M1.

### 10.10 Chunk meshing and streaming: lessons
- **Where:** `voxel.rs:2014-2886`; `renderer.rs:869-886`, `2585-3533`, `4505-4552`.
- **Keep:**
  - ghost margins;
  - off-thread builds with epochs, dropping stale results;
  - **an edited chunk keeps its old mesh until the rebuild lands**, so edits never flash a hole;
  - nearest-first and view-centre-first admission;
  - visible gaps filled coarsest-first with a reserve;
  - **never evict the wanted set** (B-21);
  - eviction that sheds gradually with a pressure valve (B-10);
  - ancestor stand-ins that suppress their drawn descendants;
  - leftovers built as one parallel batch, never sequentially (F-24).
- **Rethink:**
  - real 32³ chunks, compact vertices (the old vertex was 104 bytes), greedy merging (Andrew asked for it), pooled buffers;
  - frustum culling (only horizon culling existed);
  - exact grid-walk chunk selection (fixed-step sampling skipped chunks near face edges and punched holes);
  - the LOD choice from true 3D distance to chunk bounds (B-23 measured to the sea-level sphere);
  - band-limited coarse voxels;
  - a temporal fade on arrival, with ease inheritance (B-12);
  - crack hiding between LODs.
- **Milestone:** M1. Gate the scheduler with the convergence reel from day one.

### 10.11 Settled versus raw capture
- **Where:** `renderer.rs:4784-4957`.
- **How:**
  - A settled capture drains all streaming and zeroes the eases, failing hard if it can't settle.
  - `TRI_RAW_CAPTURE` photographs the single in-flight frame.
- **Port:** as-is (the reel contract).
- **Milestone:** M1b.

---

## 11. Sky, atmosphere, sun, stars and fog

### 11.1 Sky dome
- **Looks like:** an artistic gradient, not physical scattering:
  - a pale horizon and deeper blue zenith;
  - a warm glow toward a low sun;
  - greying under weather;
  - a fade to black with altitude;
  - a thin blue limb from orbit.

  Blessed frames: `viewer/interchange/skyreel/accepted/*.png` (tracked).
- **Where:** `shader.wgsl:2409-2568`.
- **How:**
  - `c = mix(hor (0.55, 0.70, 0.88), zen (0.10, 0.28, 0.62), pow(h, 0.55)) · (0.03 + 0.97·day)`.
  - Sunset `mix(c, (0.95, 0.55, 0.30), pow(dir·sun, 6)·(1 − smoothstep(0.05, 0.35, sun_h))·0.55·day)`. There is no afterglow once the sun is about 5° down **(inferred)**.
  - Below the horizon `×smoothstep(−0.10, 0.03, h)`.
  - Altitude `atm = exp(−alt/45 km)`; zero on the moon.
  - Limb glow `(0.30, 0.50, 0.90)·exp(−(miss − R)/20 km)·…·0.8`.
  - Murk and storm terms in §12.
- **Verdicts:** SOLAR.md non-negotiable (Austin): sun and moon appearance and their atmospheric lighting "must not degrade", enforced by the sky reel.
- **Port:** keep the gradient and colours as the M1c look. Re-derive the 45 km fade and 20 km limb against the new R; Earth-thick air on a 90 km planet looks haloed **(inferred)**. Physical scattering is optional and was never asked for.
- **Milestone:** M1c; limb glow M5; colours on the M3 panel.

### 11.2 Sun disc and halo
- **Where:** `fs_body` `shader.wgsl:2597-2607`; halo `2457-2460`; the sun is a 32×64 UV sphere (`renderer.rs:968-995`).
- **How:**
  - Disc `sun_tint (1.0, 0.96, 0.86)·3.2`, alpha `smoothstep(−0.08, 0.03, sun_h)`.
  - Halo `sun_tint·pow(dir·sun, 800)·0.50·day`.
  - The sun is 109 R at 12,190 R, giving a 0.512° angular radius.
- **Defects to avoid:** bodies draw after clouds with no cloud attenuation, so the disc shows through overcast **(inferred from pass order)**. The halo vanishes at totality, so there is no corona.
- **Port:** as-is, plus cloud transmittance.
- **Milestone:** M1c; sun surface (granulation, spots, flares) Later (D-16 accepted, deferrable).

### 11.3 Stars
- **Where:** `shader.wgsl:2376-2396`, `2560-2563`.
- **How:**
  - Cells `floor(dir·220)` with hash ≥ 0.92 hold a star, jittered 0.15–0.85.
  - Tint `mix((0.75, 0.82, 1.0), (1.0, 0.92, 0.78))`, brightness `0.25 + 1.6b²`.
  - Fade `1 − smoothstep(0.01, 0.18, sky_lum)`.
- **Defects:**
  - Stars shine through night clouds (visible in the blessed `night_stars.png`).
  - They are fixed to Neisor's surface frame, so the sky never wheels **(inferred)**. SOLAR.md's accepted "navigation by stars" needs inertial stars.
- **Port:** multiply by (1 − cloud alpha); rotate with the inertial frame.
- **Milestone:** M1c.

### 11.4 Distance fog and aerial perspective
- **Where:** `shader.wgsl:1699-1701`, `1815-1816`.
- **How:** `fog = (1 − exp(−d_km·0.0035))·exp(−alt/45)·0.7`; colour = horizon `(0.55, 0.70, 0.88)·(0.15 + 0.85·day)`.
- **Port:** rethink. On the new planet this gives about 1.5% at a 4.3 km horizon, which is invisible **(inferred)**. Make fog distance a look-panel knob, and keep "fog colour = horizon colour × day".
- **Milestone:** M1c; knob M3.

### 11.5 Valley mist and dawn fog
- **Looks like:** low mist in humid hollows around sunrise, a broad bank in saturated storms, peaks clear above a flat ceiling, a veil at the horizon when you're inside it.
- **Where:** `shader.wgsl:1786-1814`, `2467-2493`; `weather.rs:1738-1773` (`dawn_window`); `renderer.rs:3720-3771`.
- **How:**
  - Dawn window from the sunrise hour angle (`cos H = −tan(lat)·tan(dec)`), 2 solar hours wide.
  - Concavity `(coarse − detailed elevation)/120 m`.
  - `mist = (1 − exp(−d·0.85·max(valley, broad)))·below_ceiling(0.24 km)`, capped at 0.88.
  - Colour `mix((0.55, 0.62, 0.66), (0.48, 0.51, 0.45), 0.35·cover)·(0.18 + 0.82·day)`.
- **Port:** rethink. A 240 m ceiling is twice the new planet's tallest peak. Restate it in unscaled metres (about 3.5 m), or compute concavity from blocks. Keep the dawn logic and colours.
- **Milestone:** M5.

---

## 12. Clouds, weather and precipitation

All of this is one stateless, seekable system: a pure function of (seed, direction, weather time). The knobs are in `weather.rs:18-246`, with a JSON override (`viewer/assets/weather_tuning.json`, tracked) validated as a whole: any bad value means all defaults (W-8). WEATHER.md: "If Andrew wants to art-direct a value, it must be a knob."

### 12.1 Climatology bake (`weather.bin`)
- **Where:** `scripts/bake_weather.py` (166 lines); loader and sampler `weather.rs:840-1169`.
- **How:**
  - Monthly planetgen fields are Fourier-fitted at month centres as **Cartesian (a, b) coefficients**, because phase can't be interpolated:
    - temperature k = 1;
    - precipitation and clouds k = 1 + 2 (this keeps monsoons bimodal);
    - wind as the annual mean;
    - sea ice as 12 monthly layers.
  - 26 layers, 256² edge-inclusive per face, k = 3 IDW, magic `WEA3`, about 21 MiB.
  - Runtime `mean + a1·cos 2πt + b1·sin 2πt + a2·cos 4πt + b2·sin 4πt`.
- **Port:** as-is in concept; fold it into the M1-bake. Keep the per-texel reconstruction RMS as a fidelity gate.
- **Milestone:** M5 (needed for seasons and weather).

### 12.2 Synoptic weather field
- **Looks like:** live cover, rain, snow fraction, humidity and storminess anywhere, drifting with the climatological wind. The gradations (clear, scattered, broken, overcast, storm) are continuous, never binary (Austin).
- **Where:** `weather.rs:1582-1736`.
- **How:**
  - **Advection:** exact great-circle (Rodrigues) upstream along the wind, `synoptic_speed` 150. The earlier `normalize(dir − drift)` froze fronts after about a simulated year.
  - **Noise terms:**
    - `synoptic = fbm(adv, 3 oct, f 40)`;
    - `meso = fbm(1.6× drift, 2 oct, f 320)`;
    - `planetary = fbm(0.55× drift, 2 oct, f 5)`.
  - **Cover:** `raw = clim + 0.32·planetary + 0.55·synoptic + 0.18·meso + 0.48·cyclone + 0.38·front`; `cover = smooth01((raw − 0.25)/0.60)`, then arms and eye act after saturation.
  - **Precipitation:**
    - `wetness = clamp(precip_mm/220, 0, 1.5)`, so a desert overcast stays dry;
    - `precip = clamp(over^1.6·wetness + …)`.
  - `snow_frac = 1 − smooth01((T + 1.5)/3)`.
- **Verdicts:** D-8 (Andrew): "I would use the overall precipitation statistic, however some more interpolation can be done at a smaller scale to favor rain in crevices over peaks" (shipped as §12.8). WEATHER.md W-MOTION: Andrew "All agreed upon".
- **Port:** keep the design. Rescale speeds: drift divides by R, so a 10 m/s wind ×150 laps the new planet in about 6 minutes **(inferred)**. Keep frequencies planet-relative so storm belts follow planetgen's climate.
- **Milestone:** M5.

### 12.3 Cyclones, spiral arms, fronts
- **Looks like:** up to 4 seeded storms born in stormy belts. Each has:
  - a ragged clear eye;
  - a dense eyewall;
  - rotating logarithmic rain bands;
  - a curved comma-tail front.

  Each grows, matures and decays, and is reborn elsewhere. The deck rotates and latitude bands slosh.
- **Where:** `weather.rs:1183-1580`; GPU twin `shader.wgsl:1941-2053`.
- **How:**
  - **Systems:** 2 by default (max 4); life 6,300 s; `lifecycle = smooth01(age/0.18)·(1 − smooth01((age − 0.78)/0.22))`.
  - **Tracks:** 8 candidates per life at ±20–52° latitude; the best scores `0.72·cloud + 0.28·wet`; drift east at 0.35 km/s.
  - **Core wrap (bounded shear law):** `tanh(spin·age/max_wrap)·max_wrap`, max 120°. An unbounded `rate·t` shredded the deck into threads.
  - **Loads:** eye `1 − smooth((rn − 0.04)/0.14)`; eyewall at rn 0.28 ± 0.16.
  - **Arms:** `cos(n·az + twist(3.0)·ln rn − phase)` sharpened to `w·|w|`; 1.2°/s, which gives one visible revolution per 10 weather-minutes.
  - **Comma tail:** 90 km sharp leading edge, 360 km trailing smear.
  - **Radius:** 950 km. The tracked override sets `cyclone_arm_strength` to 0.75.
  - Structure shifts **cloud-formation thresholds**, not alpha.
- **Verdicts (Andrew, WEATHER.md):**
  - the density wave is his design;
  - alpha-carving read as "cartoony" ghost clouds;
  - the eye has about zero cells, ringed dense;
  - a straight front read as a "slab", hence the comma.

  Austin: the arms must visibly revolve.
- **Port:** keep the math and the bounded-shear rule. Rescale: 950 km is 10.5 rad on the new planet, so use the same fraction (about 10 km) or tune by eye. Budget its cost (about 5% on the orbit bench, R-7).
- **Milestone:** M5.

### 12.4 Synoptic raster
- **Where:** `weather.rs:654-838`; `shader.wgsl:2113-2160`.
- **How:**
  - 6 × 64² RGBA8 (cover, precip), rebuilt per 2 s weather-time bucket on rayon.
  - The shader samples it with a **smoothstep-warped bilinear** lookup (C1, no creases). A cell-constant jitter had etched a 12 km grid into overcast (Austin's report).
  - The map's "Clouds now" layer reads the same bytes.
- **Port:** as-is; drop the pin-compat scalar bypass.
- **Milestone:** M5.

### 12.5 Three cloud shells
- **Looks like:** three concentric layers with real parallax:
  - high: pale, slightly blue fibrous cirrus;
  - middle: broken cumulus with bright crowns that darken as rain builds;
  - low: dark rain bellies under the middle masses.

  Type follows weather (low cover favours cirrus, rain brings the low base). Lit per hit point and warm toward a low sun.
- **Where:** `shader.wgsl:1838-1908`, `2055-2287`, `2533-2559`; knobs `weather.rs:71-91`, `198-209`.
- **How:**
  - Ray/sphere hits at 1.8, 3.8 and 8.2 km above ground; trilinear integer-hash value noise, 2–3 taps per layer, no volume march.

    | Layer | Formation threshold | Presence | Density |
    |---|---|---|---|
    | Cirrus | 0.57 − 0.035·cold − s, on an anisotropic domain | (0.16 + 0.62(1 − cover))·…·smoothstep(0.035, 0.22, cover) | 0.32 |
    | Cumulus | 0.66 − 0.25·cover − 0.05·precip − 0.24·arm − 0.30·wall − 0.15·front + 0.85·eye | — | 0.82 |
    | Low base | 0.61 − 0.09·cover − 0.13·precip − … | max(smoothstep(0.12, 0.70, precip), 0.38·smoothstep(0.82, 0.98, cover)) | 0.92 |

    The cirrus presence gate keeps clear lanes clear (B-8).
  - **Colours × (0.10 + 0.95·day):**
    - low `mix((0.82, 0.85, 0.90), (0.24, 0.27, 0.34), heavy)`;
    - mid `mix((0.97, 0.98, 1.0), (0.39, 0.42, 0.49), heavy)`;
    - cirrus `mix((0.93, 0.96, 1.0), (0.75, 0.82, 0.92), 0.18·cover)`;
    - sunset toward (0.98, 0.62, 0.38).
  - The W-MOTION pass-1 warp, shear and morph was **reverted** as a "regression zoo".
- **Verdicts:** D-6 (Andrew): "clouds that lean on the realistic side … change over time and correlate with weather … different types of formations … extended into the third dimension, with potentially multiple layers … can be partially faked based on a 2d rendering." The three shells are the shipped answer.
- **Port:**
  - Keep the altitudes in km: clouds don't shrink, and the low shell must clear the world top (validate it).
  - **Restate cell size in km** (about 10–30 km lattices). The unit-sphere scales would give 100–300 m cells on the new planet.
  - Decks will curve visibly toward the horizon (the 8.2 km shell's horizon is about 38 km); check that with Andrew.
- **Milestone:** M5.

### 12.6 Orbital cloud cap and compositing
- **Where:** `shader.wgsl:1759-1785`, `2349-2374`.
- **How:**
  - Premultiplied low → mid → high stack.
  - `alpha = min(acc, 0.55)·smoothstep(8.2, 15 km, cam_alt)`; the cap applies after stacking.
  - Cloudy ground is darkened ×(1 − 0.10·alpha·day).
- **Verdicts:** hard rule: "Clouds must never fully hide the ground from orbit."
- **Defect:** from camera altitudes of about 1.8 to 8.2 km, terrain pixels get no cloud overlay, so you can't see the low deck below you **(inferred)**.
- **Port:** keep the cap as a validated knob. Composite the shells against scene depth at all altitudes, and drop the 1/4096 quantisation.
- **Milestone:** M5.

### 12.7 Cloud shadows
- **Where:** `shader.wgsl:2301-2347`.
- **How:** cast to the sun through each shell with the same layer function; `shadow = 1 − Π(1 − α)`; direct light ×(1 − 0.35·shadow).
- **Port:** as-is. Consider a shadow texture, since each pixel pays three cloud evaluations.
- **Milestone:** M5.

### 12.8 Ground weather: snow dusting, rain darkening, crevice rain
- **Where:** `shader.wgsl:1562-1610`.
- **How:**
  - `t_pix = T_cam − 6.5 °C/km·(pixel_elev − ground_ref)`. The reference is the ground under the camera; lapsing from the eye made the snowline recede as you climbed.
  - Dither: two camera-anchored integer-lattice value noises (25 m and 8.3 m).
  - `dust = cold·(0.45 + 0.55·snow·precip)`, toward (0.88, 0.90, 0.94).
  - `rain = precip·(1 + 0.18·(2·concavity − 1))·(1 − snow)`, ground ×(1 − 0.30·rain). That is D-8, as at most ±18% redistribution.
  - Water and ice tops are excluded (V-9).
  - All camera-anchored responses fade with `wcam = 1 − smoothstep(8, 40 km, cam_alt)`. Without it, the whole hemisphere pulsed from orbit (Austin's "polar flicker").
- **Port:** keep the dither, colours, D-8 and wcam. The lapse must use unscaled height, or 120 m peaks are only 0.8 °C colder **(inferred)**. Real snow-layer blocks are a later option (D-9; W5 "snow depth as voxel overlay").
- **Milestone:** M5.

### 12.9 Storm-edge gloom
- **Where:** `renderer.rs:3773-3816`, `weather.rs:1775-1817`, `shader.wgsl:2436-2452`, `1742-1758`.
- **How:**
  - 8 compass probes at 650 km; first-order fit of the load gradient.
  - The sky darkens up to 0.62·0.32 toward an approaching storm, with a green-olive cast (0.37, 0.42, 0.29) at 0.08.
- **Port:** rescale 650 km to about 7 km (a horizon-relative distance).
- **Milestone:** M5.

### 12.10 Precipitation particles
- **Where:** `shader.wgsl:2643-2736`; `renderer.rs:3712-3752`.
- **How:**
  - A camera box of radius 45 m and half-height 38 m.

    | | Rain | Snow |
    |---|---|---|
    | fall speed | 9.5 m/s | 1.6 m/s |
    | sway | ±0.4 m | ±3.5 m |
    | half-length × half-width | 0.40 × 0.04 m | 0.06 × 0.06 m |
    | alpha | 0.16 | 0.85 |
    | colour | (0.72, 0.78, 0.88) | (0.97, 0.98, 1.0) |

  - Fall direction tilted by wind.
  - Count `precip·(1 + 0.18·concavity·(1 − snow))·9000`.
  - Off underwater and off-planet.
  - A degenerate cross product when looking straight down made a "100 m garbage slab"; that case is handled.
- **Port:** keep the sizes (metre-scale). **New need:** occlusion so rain doesn't fall in caves or under roofs (a highest-opaque-block map) **(inferred)**.
- **Milestone:** M5.

### 12.11 Never built (accepted backlog)
- Lightning and thunder delay, rainbows, aurora (D-16 accepted), dust storms, puddles, weather audio, and the W-MOTION evolution terms (agreed but reverted).
- **Milestone:** Later.

---

## 13. Time, orbits, the moon in the sky, eclipses

### 13.1 One absolute clock
- **Where:** `renderer.rs:1930-2098`; `main.rs:2306-2329`; `orbits.rs:587-637`; `ui.rs:1641-1721`.
- **How:**
  - `t = anchor_abs + (render_t − anchor_render)·scale + offset`. Changing the scale folds "now" into the anchor, so there is no jump; seeking sets the offset.
  - Speed ladder `[1, 10, 60, 600, 3600]×` on `[`/`]`.
  - The harness drives render time at a fixed 60 Hz, so captures are byte-identical.
  - Calendar: day = `day_length_s`, month = moon period (7 d), year = Neisor period (84 d) = 12 months.
  - Time panel: "Now: y/m/d hh:mm, season", "Travel to" fields, a time-of-day slider, "Travel (stay here)", speed buttons. Austin's request was to "verify seasons easily".
  - All of it is disabled while connected to a server (D-17).
- **Verdicts:** D-5; SOLAR.md (Austin): "every body position must be a pure O(1) function of (t, seed/params)"; SOLAR.md: "the D-5 calendar IS the moon", accepted.
- **Mismatch:** the UI's `calendar().season_frac` is day-of-year/year, while weather seasons use the orbital mean anomaly with a 0.45 epoch offset. They differ by about 0.45 **(inferred)**. Unify them.
- **Port:** as-is, with a 1,440 s day (§0.6). At 24-minute days the dawn ramp passes in about 53 s real time **(inferred)**; check with Andrew.
- **Milestone:** M1c (clock, ladder, seek); calendar and Travel UI M5 (or M1c, it is cheap).

### 13.2 Stateless Kepler hierarchy
- **Where:** `orbits.rs:33-103`, `150-204` (defaults, mirrored in tracked `viewer/assets/solar_tuning.json`), `398-485`.
- **How:**
  - Sun at the root; Neisor orbits the Sun; the moon orbits Neisor.
  - Eight Newton steps on Kepler's equation.
  - Inertial to fixed: `Rz(rotation_phase − 2πt/day)·Rx(tilt)`.
  - **Semi-major axes are stored in Neisor radii**, so angular sizes and the eclipse calendar survive a radius change unchanged.

  | Body | Radius | Orbit | Gravity |
  |---|---|---|---|
  | Sun | 109 R_N | root | 274 |
  | Neisor | 1 R_N | a = 12,190 R_N, e = 0.035, i = 0, ω = 42.92°, M₀ = 162°, 84 d | 9.81 |
  | Moon | 0.27 R_N | a = 13.86 R_N, e = 0.08, i = 4.8°, Ω = 225.01°, ω = −122.67°, M₀ = 90°, 7 d | 1.635 |

  - Neisor: axial tilt 23°, rotation phase 15.8°.
  - Tints: sun (1.0, 0.96, 0.86), halo 0.50, moon (0.86, 0.89, 0.97), copper (0.76, 0.25, 0.08).
  - The moon is distance-compressed to a 1.203° angular radius (about 4.6× the real Moon, 2.35× the Sun), so totality is easy.
  - Inclination is the main eclipse-rate control.
  - Season = Neisor mean anomaly / 2π; `weather season FRAC` retargets M₀, so everything moves together.
  - The synodic month is about 7.64 d versus the calendar's 7 **(inferred)**.
  - The tuning file is validated as a whole.
- **At the new scale (inferred):** moon R ≈ 24.4 km at about 1,255 km; Neisor seen from the moon about 4.2° in radius.
- **Port:** as-is, nearly verbatim.
- **Milestone:** M1c for the sun direction and seasons; the moon terms in M5 (or M1c, see §13.4).

### 13.3 Pinned-sun rule
- **Where:** `renderer.rs:2020-2055`.
- **How:** a pinned sun (`sun LAT LON` or local-noon mode) rotates the whole frame **about Neisor's centre**, never about the observer. Rotating about the observer produced phantom eclipses.
- **Port:** as-is. **Milestone:** M1c.

### 13.4 The moon seen from Neisor
- **Where:** `shader.wgsl:1049-1140` (`vs_moon`/`fs_moon`), `2512-2532` (halo).
- **How:**
  - `lit = mix(lambert, √lambert, 0.12·smoothness)`, so maria have softer terminators.
  - Brightness `mix(0.05 + 0.95·lit, 0.22 + 0.18·lit, lunar_shadow)`.
  - Visibility `max(1 − 0.9·day, solar_occlusion)·smoothstep(−0.06, 0.06, moon_alt)`: a 10% daytime moon.
  - Halo `(0.55, 0.62, 0.80)·pow(dot, 2600)·0.30`.
  - **Earthshine was lost**: the old shader moon had it; now there is only a constant 0.05 floor.
  - **The moon is not tidally locked** **(inferred)**: its axes are fixed in Neisor's rotating frame, so the visible face turns about 180° between moonrise and moonset.
- **Verdicts:** Austin's non-negotiable (must not degrade); the sky-reel pose `moon_zenith`.
- **Port:** as-is, plus tidal lock (libration comes free from e and i) and Neisorshine scaled by phase.
- **Milestone:** M1c recommended: the orbit code is stateless, nights look flat without the moon, and the sky reel pins it. Otherwise M5 per PLAN.

### 13.5 Solar eclipse
- **Where:** `orbits.rs:487-565`; WGSL twin `shader.wgsl:1142-1176`.
- **How:**
  - Exact two-circle lens overlap ÷ solar disc area, evaluated **per pixel**, so the penumbra sweeps across Neisor; a conservative whole-planet gate first.
  - `day ×= 1 − occlusion`, and the sky uses the camera's occlusion. At totality the sky goes night and stars appear.
  - The moon mesh depth-covers the sun.
- **Verdicts:** D-16 (Andrew): "solar eclipses should reduce the light coming from the sun proportional to the occluded area until the sky becomes night-like."
- **Port:** as-is, about 60 lines.
- **Milestone:** ships with the moon (SOLAR.md: "a physical moon between sun and Neisor WILL occlude").

### 13.6 Lunar eclipse (copper moon)
- **Where:** `orbits.rs:567-585`.
- **How:** the fraction of the sun hidden by Neisor as seen from the moon's centre; `fs_moon` mixes the moon tint toward copper (0.76, 0.25, 0.08) by that fraction.
- **Gaps:** one uniform value, so the umbra edge never crosses the face; moonlight on Neisor is not dimmed.
- **Verdicts:** D-16: "Copper-tinting is a nice feature."
- **Port:** as-is, plus an optional per-pixel umbra and moonlight dimming.
- **Milestone:** with the moon.

### 13.7 Eclipse frequency
- **Where:** locked by test, `orbits.rs:751-785`.
- **How:** per 84-day year, 2 solar contacts (one total, one about 29% partial) and 9 lunar contacts, irregularly clustered.
- **Verdicts:** D-16: "much greater rate than on Earth, but … irregular, that's somewhat part of the charm; use the orbital parameters to tune this factor."
- **Port:** as-is; it is R-relative, so it is unchanged **(inferred)**.

### 13.8 Neisor seen from the moon
- **Where:** `shader.wgsl:2463-2511`.
- **How:** a black airless sky; Neisor with its true terminator (per-pixel day above 2.5–40 km), the blue limb glow, and the moon's eclipse shadow visible on it.
- **Port:** as-is; rescale the 20 km limb falloff.
- **Milestone:** M5.

### 13.9 Banked ideas (not implemented)
- Trojan asteroid ("good test to add").
- Tides (deferred behind water levels).
- Comet and meteor craters (deferred).
- Sun spots, flares and auroras (accepted, deferrable).
- Lunar water and ice deposition and resource reservoirs (Andrew's spec).
- Planet shading instead of whole-screen day/night (done).
- **Milestone:** Later.

---

## 14. Moon terrain

**Source of truth:** `viewer/src/moon.rs` (2,805 lines), Andrew's two gitignored spec files (§0.5), and SOLAR.md:101-121 ("Moon final form - the r3 roadmap"). **The r3 changes (maria and rays) were never implemented**; `moon.rs` was last changed in r2 (`9548999`) and the rocks commits.

### 14.1 One terrain law
- **Where:** `moon.rs:1568-1649` (`sample`), `70-84`, `657-684`, `1651-1840`.
- **How:** `sample(dir)` returns `height_ratio` (a fraction of R), `albedo`, `smoothness`, `ray` and `debris`. Mesh, voxels, materials, rocks, the map and probes all call it.
  1. Highland base, then maria.
  2. Crater octaves 0..9, coarse to fine, in hash age order.
  3. The Tycho-size ray cohort is inserted after octave 4, so fine craters overprint the rays.
  4. Clamp height to [−0.014, 0.010] R and albedo to [0.15, 0.96].
  - Hard budget: 99 cell visits per sample; octaves 0–4 are precomputed.
  - Splitmix hashing; batch sampling bit-identical to scalar.
- **Verdicts:** SOLAR.md (Austin): "One truth, two renderers stays law on the moon."
- **Port:** as-is. Replicate `noise.rs` bit-exactly if you want to reuse the regression pins. **Drop the gnomonic `cube_metric`** (`1/(1 + u² + v²)`, `moon.rs:1230-1233`), which shrinks craters up to 3× toward cube corners on an equal-area grid.
- **Milestone:** M5.

### 14.2 Moon size and lattice
- **How:** the old moon had its own 2,700,000-column lattice for true 1 m cubes. That fixed Andrew's "blocks on the moon are exaggerated vertically".
- **New engine (inferred):** R ≈ 0.27 × 90.57 ≈ 24.4 km.
  - Candidate N: 35,328 (69 × 512) or 35,840 (35 × 1024; ratio 0.2734, close to the real 0.2727).
  - Relief spans [−342, +245] m, so **H_moon = 1024 is comfortable, 512 is tight**.
  - The horizon from 1.7 m is about 290 m.
- **Port:** rethink (choose N and H).
- **Milestone:** M5.

### 14.3 Highland base
- **Where:** `moon.rs:1035-1044`.
- **How:**
  - Height `fbm(4 oct, 1.15)·0.00058 + ridged(0–1, 3.2)·0.00010 + fbm(2 oct, 74)·0.000014` (in R).
  - Albedo `0.695 + 0.064·fbm(3, 2.1) + 0.020·gn(×17) + 0.017·gn(×92)`, about 0.6–0.78.
- **Verdicts:** Andrew: "the moon should be smoother than Neisor"; "base layer … very large-scale noise octaves".
- **Port:** as-is. **Milestone:** M5.

### 14.4 Maria
- **Looks like:** dark (albedo about 0.32), flat, smooth basalt plains; a near-side cluster of big maria with smaller ones on their edges.
- **Where:** `moon.rs:571-646`, `1046-1094`, `1847-1855`.
- **How:**
  - **Spawn field:** 4 Gaussian clusters at (−16, −18), (20, 24), (5, 61) plus one random, each jittered ±7° and ±9°, radius 24–36°. Combined as `1 − Π(1 − c)` plus `0.075·gn(×2.4)`.
  - **Large maria:** 3×3 cells per face, `p = 0.025 + 0.70·smoothstep(0.30, 0.80, field)`, radius 5.5–17°, albedo 0.315–0.335.
  - **Mid maria:** 7×7 cells, with a cluster-edge band term, radius 1.7–8.5°, albedo 0.325–0.347.
  - **Silhouette:** `1 + 0.24·gn(×4.5) + 0.070·sin(5b + φ) + 0.025·sin(11b − 0.4φ)`.
  - **Masks:** height uses a wide feather (0.74–1.10); albedo a tight edge (0.965–1.075).
  - **Overlaps saturate** (max mask, min albedo).
- **Verdicts (Andrew):**
  - more clustered cratons with mid maria at their edges;
  - "basalt over basalt is the same basalt", so overlaps must not darken (resolved);
  - r2: "darker, tighter, bolder";
  - **r3 (not done):** keep the centres and distribution; grow each mare so they overlap and interconnect; make them much more circular overall with much noisier fine edges; kill the straight-line, patchy look. Acceptance is the blue pen line in `current-and-target-lunar-mare.JPG`.
- **Port:** port, then apply r3. **Milestone:** M5.

### 14.5 Crater lattice and size–frequency law
- **Where:** `moon.rs:25-68`, `352-367`, `505-568`, `1191-1270`.
- **How:**
  - Octave k is a 2^k grid per face (1 … 512), occupancy 0.31; octave 0 uses `0.42 + 0.32·spawn`.
  - Jitter ±0.47 cell.
  - Radius log-uniform 0.20–0.46 of a cell; bands overlap more than 2×, so there is no "size comb".
  - Elongation `1 + 0.20·u⁷` (oblique impacts are rare).
  - Face-0 counts per octave: 1, 2, 6, 20, 86, 348, 1456, 5500, 22425, 88678, i.e. N ∝ D⁻².
- **New sizes (inferred):** basins about 5.7–7.4 km in radius; the finest about 3.5–10.5 m. That already meets Andrew's "down to tens of metres".
- **Verdicts (Andrew):** "up to lunar mare" size; "1000 times more common" small craters; "WAY more craters"; r2: de-gridded, continuous sizes and angles.
- **Port:** as-is. Decide how many octaves survive at 1 m blocks.
- **Milestone:** M5.

### 14.6 Crater profile
- **Where:** `moon.rs:1338-1471` (with q = elliptical distance, 1 at the rim).
- **How:**
  - **Depth** `r·mix(0.024, 0.132, √scale)·(0.82–1.12)`: d/D about 0.012 for basins and 0.054–0.074 for the finest.
  - **Rim radius noise:** lobes 5–14, clamped 0.78–1.22.
  - **Floor:** flat, with a slight bowl.
  - **Central peak** on octaves ≤ 2: `+depth·mix(0.48, 0.30, scale)·exp(−(q/0.21)²)` with a summit dimple `−0.15·exp(−(q/0.052)²)`.
  - **Rim:** steep inside (`exp(−((q − 1)/0.060)²)`), gentle outside (0.255) out to 1.78 r. The ejecta ramps from the crest via `smoothstep(1.0, 1.10, q)`; that is the B-3 fix for 492 m cliffs.
  - Fresh small craters have higher rims; basin rims sit near ground level.
- **Verdicts (Andrew):** "ridges, dimple in the middle for large craters"; "the interior of a crater's rim … more sudden … than the exterior"; "on larger craters … the rim … at the same level as the adjacent ground"; larger craters "rougher with a slightly irregular shape".
- **Port:** as-is, but **re-tune depth for blocks**. Real simple craters are about d/D 0.2, so fine craters at 0.06 will vanish at 1 m **(inferred)**.
- **Milestone:** M5.

### 14.7 Impact floors and floor colour
- **Where:** `moon.rs:1103-1168`, `1381-1470`.
- **How:**
  - A young crater replaces terrain inside its bowl with one local datum. An older rim crossing it is erased (99% in the r2 probe).
  - Floor colour comes from the impact centre, so a crater on a mare edge takes one colour.
  - Floors erase rays.
  - Bright rims only on the freshest ~20%.
- **Verdicts (Andrew):** "new craters … overriding the terrain at the impact site"; "crater basins on the edge of real lunar mare seem to only inherit one color"; universal bright rims "significantly curbed"; "rim contrast should come from lighting, not paint".
- **Port:** as-is. **Milestone:** M5.

### 14.8 Ray systems
- **Looks like:** about 11 rare mid-size craters throw bright straight-ish rays out to 14 radii, with a dark smooth halo just outside the rim.
- **Where:** `moon.rs:1272-1332`, `1473-1566`.
- **How:**
  - **Carriers:** 54 cells, occupancy 0.20, radius 0.55–1.25° (about 235–533 m on the new moon); basins never carry rays.
  - **Halo:** 0.99–2.12 r, pulling albedo toward `clamp(0.58·a, 0.24, 0.48)`.
  - **Lines:** 24 per crater.
    - 7 long: 7–14 r long, 0.12–0.25 r wide.
    - 17 short gap-fillers: 3.1–8.2 r long.
    - Start at 1.24–1.52 r; taper `(1 − progress)^0.72`.
    - Albedo `+= (0.96 − a)·ray·0.84`.
  - **The two terms r3 wants gone:**
    - `warp` (`:1539-1540`) makes the rays snake;
    - `broken` (`:1551-1552`) makes colour bands along each ray.
- **Verdicts (Andrew):**
  - v2 spec: rough tapering lines, small rays filling gaps, rarer, Tycho-size only, none on big craters; "ground immediately outside … darker and smoother".
  - **r3 (not done):** one consistent colour per ray; straight rays; roughness as fine granular ejection texture (much higher frequency than `gn × 180`); fill the radial gaps near the rim evenly while keeping length variance.
  - **B-6 (open):** oblong craters get circular halos and rays; oblique impacts should get butterfly ejecta.
- **Port:** port, then apply r3 and B-6. **Milestone:** M5.

### 14.9 Moon materials, regolith strata, boulders
- **Where:** `moon.rs:103-113`; `voxel.rs:149-151`, `1000-1024`, `1326-1330`, `1415-1490`.
- **How:**
  - **Class:** Ray if ray ≥ 0.08, Maria if smoothness ≥ 0.45, else Highland.
  - **Column:** top block Regolith{Highland|Maria|Ray} tinted by albedo³; 4 blocks of RegolithSubsurface (×0.72, 0.70, 0.68); then coal-dark LunarRock (×0.48, 0.47, 0.46).
  - **On screen:** ×moon tint (0.86, 0.89, 0.97). Highland is about (0.60, 0.62, 0.68), mare about (0.28, 0.29, 0.31).
  - **Boulders:** density `lerp(4e-5 highland, 1.5e-5 maria) + 5e-4·debris^0.78`, capped at 5.4e-4. `debris` is born in the crater law (rim core, ejecta, ray halo), so rocks sit on rims and aprons. The material is dust-coated RegolithHighland.
  - The moon has no water, caves, trees or weather.
- **Verdicts:** Andrew's doc defers boulder and cobble block types until textures, and rubble piles until block models and levels.
- **Port:** keep the classes, strata and placement law; colours are provisional.
- **Milestone:** M5.

### 14.10 Moon mesh tiles: drop
- **Where:** `moon.rs:1871-2096`.
- **Port:** drop; LOD the moon like Neisor. Keep the far-body shared anchor (§10.9).

---

## 15. Player movement, camera, swimming

### 15.1 Walking
- **Where:** `player.rs:15`, `83-91`, `269-367`; `voxel.rs:1798-1893`.
- **How:**
  - **Body:**
    - eye 1.8 m;
    - walk 4.3 m/s, sprint ×2.2;
    - step-up 1.05 blocks grounded, 0.05 airborne;
    - body radius 0.35 block, with 8 probe columns.
  - **Blocking rules:**
    - A probe blocks only when you move toward it (`dot > 0.1`); this fixed a deadlock.
    - Headroom must exceed eye + 0.4 m, so **the body is effectively 2.2 m tall and 2-block tunnels are impassable**.
    - A blocked move restores the pose.
  - **Great-circle motion** with heading transport (no spiral into the pole).
  - **Solid for collision:** blocks, trunks, ice, and boulders and larger.
- **Port:** rethink. Every query was single-column. Real blocks need an AABB or capsule swept against the grid. Keep:
  - the auto-step;
  - directional blocking, so you can always leave a wall you touch;
  - great-circle transport.

  Revisit the 2.2 m headroom (2 blocks is the norm). Resolve the mixed units (blocks versus metres) before the ½ m switch.
- **Milestone:** M1b.

### 15.2 Jump and gravity
- **Where:** `player.rs:75-80`, `338-366`.
- **How:**
  - Jump 5.2 m/s, giving an apex of about 1.38 m **(inferred)**.
  - Gravity per body from `solar_tuning.json`.
  - Terminal −80 m/s; snap-to-support landing; head-bump clamp; no fall damage.
  - `refresh_after_edit` resyncs grounding after an edit under you (R5-1).
- **Port:** as-is; the new planet's gravity is an unrecorded choice (assume 9.81).
- **Milestone:** M1b.

### 15.3 Swimming and the underwater view
- **Where:** `player.rs:142-153`, `340-350`; `voxel.rs:1905-1932`.
- **How:**
  - You are in water when feet + 0.9 m are below the surface.
  - Vertical velocity eases toward +3.0 m/s (Space) or −1.4 m/s (sink) at `min(6·dt, 1)`; no horizontal drag.
  - The underwater flag (eye below surface − 0.3 m) is computed in one place after every pose change (R2/R4).
  - The view mixes 60% toward (0.02, 0.07, 0.16); no rain underwater.
  - Ice is solid.
- **Gap:** climbing out at a vertical bank may only work where you can touch bottom **(inferred)**.
- **Port:** keep the numbers and the single refresh point. Use fractional water depth (M2). Upgrade to depth fog (M3).
- **Milestone:** M1b (the ocean exists in M1); M2 for levels.

### 15.4 Fly mode and cruise lock
- **Where:** `player.rs:23-28`, `206-268`; scroll `main.rs:2449-2483`.
- **How:**
  - Speed `clamp(alt·0.5/s, 20 m/s, 1500 km/s)`, sprint ×4.
  - Below 2.5 km, absolute height is held, kept 2.5 m above support and 0.8 m below ceilings (fly is not noclip).
  - Above, **the elevation lock (C-1)** holds the radius from the planet centre, freezing while riding a peak.
  - Scroll ×(1 − 0.12·notches), from 2.5 m to 80,000 km.
  - `--auto-tilt` (C-3) is opt-in.
- **Verdicts:** Andrew (`requested-changes-and-bugs.txt`, BUGS F-7):
  - C-1 "lock to elevation";
  - C-2 "velocity … roughly proportional to the radius … a limit that's pretty far away where velocity stops increasing". A first cut slowed far flight 3–8×, and "Andrew felt it within a day";
  - C-3 "default no pan, so user has full control".
- **Port:** keep the laws; make the cap and ceiling R-relative. Choose a new lock threshold, for example above world height H.
- **Milestone:** M1a (free-fly camera).

### 15.5 Freecam and body focus
- **Where:** `camera.rs:265-437`; `main.rs:1788-1927`.
- **How:**
  - C cycles Neisor → moon → sun → freecam.
  - Focus places the camera at 3 radii; `realign` translates with the body each frame, so the pose survives timeskips.
  - Freecam: 6-DOF, Space up, Ctrl down, Q/E roll at 1.2 rad/s; speed `clamp(0.5·nearest altitude, 0.02, 1500) km/s`.
  - G/F switch walk/fly in place without refocusing. Refocusing had "yanked moonwalkers to space" (Andrew, twice).
- **Verdicts:** Andrew's spec: freecam with full rotation, Q/E roll, focus on other bodies.
- **Port:** as-is.
- **Milestone:** freecam M1a; body focus M5.

### 15.6 Camera core
- **How:**
  - Yaw/pitch/roll in the tangent frame (yaw 0 = north).
  - **Pitch clamped to ±1.50 rad on every path**, including programmatic placement. B-5: exactly −90° degenerated the basis and spun the view.
  - Mouse 0.0022 rad per count captured, 0.0032 per pixel when drag-looking.
  - First LMB captures; Esc releases, a second Esc quits.
- **Port:** as-is. **Milestone:** M1a.

### 15.7 Teleport
- **Where:** `player.rs:102-135`; `main.rs:1565-1753`.
- **How:**
  - `teleport(lat, lon, alt?)`, clamped 2.5 m to 80,000 km.
  - Photo restore is exact and seed-gated (F-21), with optional time and weather restore.
  - Moon: "landing" (surface + eye, walk) or "flyby" (25 km), focusing the moon so it doesn't drift away (Andrew's report).
  - Non-finite input rejected (F-19).
- **Port:** as-is; derive ground from voxel support.
- **Milestone:** M1a (CLI/harness); M1c (map).

---

## 16. Block editing and torches

### 16.1 Break and place
- **Where:** `player.rs:374-446`; `voxel.rs:47-85`, `1938-2000`; `main.rs:830-990`, `1246-1303`.
- **How:**
  - LMB breaks, RMB places; reach 8 m on foot, 60 m flying.
  - Picking marches the ray in 1/3-block steps; the hit is the first point below its column's surface, and the previous column is the air column.
  - **The world's entire mutable state was a sparse map of per-column height deltas**: break is −1 on the hit column, place is +1 on the previous one.
  - There is no block choice: a placed block takes the column's strata.
  - The self-placement guard refuses blocks that would overlap your body (R1).
  - Remeshing covers ±4 columns across faces.
  - **Saves:** `assets/edits_seed{seed}.bin`, format EDT1 (`"EDT1"`, u32 count, then 25-byte `[u8 face, u64 ci, u64 cj, i64 dh]` records). Rewritten atomically on every edit, with each record validated on load (F-19). The moon has a separate file.
- **Verdicts:** Andrew (`development-direction.txt`): "blocks can only be placed or broken in columns. A true block palette, chunk loading and saving system, and block placement system should be implemented. Chunks should only save if a block is placed or broken."
- **Port:** rethink completely. The column model cannot dig a horizontal tunnel, so M1's done-criterion is impossible with it. Keep:
  - face-aware placement (now an exact voxel DDA returning block plus face);
  - separate walk and fly reach;
  - the self-placement guard (as an AABB test);
  - "resync derived player state after any edit";
  - validated, atomic saves;
  - strata anchored to natural ground;
  - neighbour remeshing.

  The hotbar is new.
- **Milestone:** M1b.

### 16.2 Torches
- **Looks like:** R toggles a small crossed wooden post with a glowing flame on the column you aim at. It casts a warm flickering pool about 11 m across. Austin called the torch-lit night shot "the game's first cozy screenshot" (ITERATION_LOG).
- **Where:** `player.rs:448-471`; `voxel.rs:87-89`, `2840-2883`; `renderer.rs:295-306`, `2391-2435`; `shader.wgsl:1674-1698`; file `assets/torches_seed{seed}.bin` (TRC1, 17-byte records).
- **How:**
  - Two diagonal quads 0.62 blocks tall; wood (0.25, 0.15, 0.06) to flame (1.0, 0.80, 0.42).
  - The flame is emissive with flicker `0.86 + 0.22·sin(9t + dot(pos, (310, 470, 130)))`.
  - The nearest 16 torches become point lights: radius 11 m, `a = clamp(1 − d/11 m)`, `+= albedo·(1.0, 0.70, 0.36)·a²·(0.88 + 0.18·sin(9t + hash(face, ci, cj)))`.
  - The flicker phase is tied to the torch's identity, so it is deterministic (F-20).
  - Torches ride edits (they always stand on the column top). Neisor only; offline only in multiplayer.
- **Port:** rethink. It becomes a placeable non-full block at (face, i, j, k), wall-mountable, lighting through real block-light propagation. Keep the warm palette, quadratic falloff, deterministic flicker and the persisted set.
- **Milestone:** M3 (a non-full block and a lighting look Andrew should see). The carry light (§10.4) bridges M1c.

---

## 17. UI

### 17.1 Input routing
- An open popup owns all input; window focus loss clears held keys; key auto-repeat is ignored (F-20).
- Esc backs out one layer at a time: pending rebind → lightbox → delete-confirm → selection → popup → mouse capture → quit.
- **Port:** as-is. **Milestone:** M1b.

### 17.2 Keybindings and the Controls window (F1)
- **Where:** `viewer/src/keybindings.rs` (defaults 151-175, tests 333-444); window `ui.rs:2653-2830`.
- **How:**
  - 21 rebindable actions, persisted to `assets/keybindings.json`.
  - The table is always complete and injective: rebinding to a taken key **swaps** them. Esc is reserved.
  - Missing actions are filled from defaults; duplicates or bad JSON mean all defaults.
  - Changed rows are tinted; "Reset to defaults".

  | Action | Key | Does |
  |---|---|---|
  | Move | W A S D | |
  | Sprint | L-Shift | the window says 4x; it is ×2.2 when walking |
  | Jump / swim up / freecam up | Space | |
  | Descend (freecam) | L-Ctrl | |
  | Roll | Q / E | |
  | Walk / Fly | G / F | |
  | Cycle focus | C | |
  | Torch | R | |
  | Map | T | |
  | Photo | P (Shift+P = raw) | |
  | Sync delta | V | |
  | Stream level | F9 | |
  | Record | F10 | |
  | Time slower / faster | [ / ] | |
  | Controls | F1 | |

  Fixed controls: LMB (capture, then break), RMB (place), wheel (fly altitude), drag-look, Esc.
- **Port:** as-is; add a hotbar (1–9, wheel). Drop V and F9.
- **Milestone:** M1b.

### 17.3 Photo map and teleport window (T)
- **Where:** `ui.rs:1-264`, `266-1148` (map synthesis), `1226-2584`.
- **How:**
  - An egui window auto-fit to 92% × 90% and re-centred on resize (Austin's reports).
  - **The map:**
    - an equirectangular map re-synthesised from the bake rasters for the exact view, zoom 1–180×, cursor-anchored, slippy-map panning;
    - base layers Biomes / Temp / Precip (current season) / Elevation (hypsometric tint, about 12 adaptive contour bands, every 4th an index line);
    - toggles: relief, rivers (vector, thicker by flow), lakes (liquid blue, frozen pale), "Clouds now", wind streamlines, markers.
  - **The photo roll:**
    - every `interchange/shot_*.png` becomes a marker: blue (80, 200, 255), yellow when selected, orange-red when checked; plus green "you are here" and a red cross for a free target;
    - 560 px preview and a lightbox;
    - editable Title and Notes saved into the sidecar;
    - "Delete N…" moves files to `interchange/trash/` transactionally.
  - **Teleport:**
    - to a photo, a map point or typed "lat lon [alt]";
    - optional "Restore photo's time of day", which also restores weather.
  - **Panels:** the time panel (§13.1) and the multiplayer row.
- **Verdicts:** D-16: "the teleport map must cover the moon" (done, §17.4).
- **Port:** as-is; re-synthesise from the new bake.
- **Milestone:** M1c (typed teleport M1b).

### 17.4 Moon map tab
- **Where:** `ui.rs:1146-1221`.
- **How:** an equirectangular map from the exact moon law: albedo × hillshade under a fixed NW light (−0.34, 0.43, 0.835), `clamp(0.68 + 0.39·hill, 0.52, 1.07)`, capped at 640×320. A click lands you.
- **Port:** as-is. **Milestone:** M5.

### 17.5 Screenshots and JSON sidecars
- **Where:** `main.rs:562-720`, `1340-1397`, `2339-2358`.
- **How:**
  - P settles the frame first (byte-stable evidence). Shift+P captures the live frame ("the only lens that shows what you actually saw mid-flight").
  - Filename `shot_lat{.3}_lon{.3}_alt{.3}km_yaw_pitch[_raw].png`.
  - **The sidecar records:**
    - pose (including `ground_km`), body and body-local pose;
    - sun (direction, pinned flag, day phase, day length);
    - seed, world and mode;
    - weather (on, pinned, t, season, cover, precip, snow, humidity, temperature);
    - solar state (t, sun_km, moon_km, occlusion, lunar shadow);
    - build stamp;
    - frame_ms (avg, p95, draw CPU).
  - Moon attribution follows the bound body (Andrew's misplaced-photo report).
- **Port:** as-is; it is the core of "no repro, no fix". Add block size, H, generator version and an edit-state hash.
- **Milestone:** M1b.

### 17.6 F10 session recordings
- **Where:** `main.rs:2254-2305`, `2628-2643`; replay `play.rs:1169-`.
- **How:**
  - A 10 Hz `pose lat lon alt yaw pitch` track written to `interchange/recordings/rec_NNN.play`, with a mode and weather-time header.
  - The harness replays it, live with `TRI_POSE_LIVE`, or as video frames with `TRI_POSE_RENDER`.
- **Gaps (inferred):**
  - It is a **camera track**, not an input replay: edits, physics and roll are not reproduced.
  - It records render time rather than absolute time, and not the time scale.
  - The app ran variable dt (capped at 0.1 s), so inputs could not have replayed anyway.
- **Port:** keep F10 as the human trigger and the pose track for convergence work. It is superseded by M2's snapshot-plus-inputs replays, which need a **fixed physics tick in the app**.
- **Milestone:** M1b (pose track); M2 (input replay).

### 17.7 HUD
- **Where:** `main.rs:1933-1981`.
- **How:** the window title was the HUD: `Neisor [build] — fly|walk | lat lon alt | N fps (avg ms, p95 ms) | MULTIPLAYER`. There was **no crosshair, block highlight or in-game text**.
- **Port:** replace with a real overlay.
- **Milestone:** fps, p95 and pose in M1a; crosshair and block outline in M1b.

### 17.8 Tuning files and the CLI
- **Tuning files** (`assets/`): `solar_tuning.json`, `weather_tuning.json`, `keybindings.json`. Each is validated as a whole: invalid means all defaults, loudly.
- **CLI** (`main.rs:22-184`): `--capture PATH` (headless one frame plus sidecar; what `repro_shot.py` emits), `--lat --lon --alt --yaw --pitch --size --sun-lat/lon --day-len --weather off|live|C,P --weather-time --join`, and others. Only finite numbers are accepted.
- **Port:** as-is; this is the precedent for M3's look-panel file.
- **Milestone:** capture CLI M1a; look files M3.

---

## 18. Multiplayer (MP1)

- **Looks like:** two players on one server share the clock, block edits and presence, shown as tinted box avatars with name tags.
- **Where:** `viewer/multiplayer/src/` (protocol, clock, journal, invite, identity), `viewer/server/src/main.rs`, `server/src/bin/triangulum-client-sim.rs`, `viewer/src/net.rs`; avatars `renderer.rs:1804-`; `docs/archive/MULTIPLAYER.md`.
- **How:**
  - **Premise:** because the world is a pure function of (seed, position, time), only three things sync: **clock, edit journal, presence**. There is no chunk streaming.
  - **Protocol:** JSON over WebSocket, `PROTOCOL_VERSION = 1`. Messages: Hello, Welcome, Refusal, EditRequest, Edit, Presence, ClockCommand, ClockEvent, join/leave, Ping/Pong, Error.
  - **Edits:** EditRequest carries an **absolute** column value, so server ordering is last-write-wins.
  - **Identity:** `WorldIdentity` = seed + build hash + SHA-256 of the bakes and `solar_tuning.json`. A mismatch is refused loudly on both ends.
  - **Server:**
    - Hello within 10 s; constant-time token compare.
    - Welcome carries the full journal.
    - Edit token bucket 30/s, burst 90.
    - EDJ2 journal: 42-byte records, appended and fsynced, torn tail truncated loudly; imports single-player EDT1 on first start.
    - Ping every 10 s, timeout 35 s; queue 256; inbound cap 128 KiB.
    - Operator console: `seek | scale | status | players | kick | quit`.
    - Clients' clock commands are refused (D-17).
  - **Clock:** a server anchor plus sequence number; the client slews onto server time over 2 s.
  - **Client:**
    - presence at about 15 Hz;
    - remote poses lerp over 100 ms;
    - a gap in the edit sequence means "MULTIPLAYER DIVERGENCE" and a disconnect;
    - offline state is snapshotted and restored on leave.
  - **Avatars:** torso and head boxes, a 5×7 bitmap name tag, drawn 0.25 m–10 km, at most 16.
  - **Invites and deployment:** `triangulum://host:port/#token`; a Cloudflare tunnel to `wss://triangulum.dieorwrite.net`, whose ingress must be `127.0.0.1:7777`, not "localhost".
  - Integration test: `scripts/mp1-integration.ps1`.
- **Verdicts:** D-17 (open): who may bend time. The default is the operator only; options include op-only, per-player, vote, or "anyone (chaos mode, which honestly might be fun for you two)". Torches stayed offline-only. R-5c (token in the URL) is open.
- **Port:** as-is: the protocol shape, identity gate, LWW edits, fsynced journal, clock slew and hardening constants. Edit payloads become (face, i, j, k, block id) or chunk deltas. Keep the net layer absent from harness runs.
- **Milestone:** Later. **But design M1b saves and M2 replays so the journal and identity fit them**: cheap now, expensive later.

---

## 19. Verification tooling

### 19.1 The play harness
- **Where:** `viewer/examples/play.rs`; its header (lines 1-169) is the command reference.
- **How:**
  - `play SCRIPT.play [--out DIR] [--size WxH]` runs the game's own player and renderer headless at a fixed 60 Hz, in a clean world with no saved edits.
  - Output goes to `interchange/runs/<stem>/`: PNG frames, JSON state sidecars (pose, grounded, underwater, support, ceiling, water surface, terrain `{h, water, sea, lake, temp, frozen, koppen, river_dist}`), and a byte-stable `run.log`.

  | Group | Commands |
  |---|---|
  | Placement | `teleport`, `moonland`, `look`, `turn`, `roll`, `mode walk\|fly`, `focus` |
  | Motion and time | `hold KEYS SECONDS`, `tap space\|lmb\|rmb\|r`, `wait`, `pose`/`posehz`, `glide`, `dwell`, `bench N` |
  | World state | `sun LAT LON` (global; omit it in surveys), `weather off\|live\|pin\|time\|season`, `stream`, `voxels` |
  | Inspection | `shot` (settled), `state`, `probe LAT LON`, `pick PX PY` (inverts the renderer's own matrix), `log`, `assert FIELD OP VALUE` |

  - `assert` supports `== != < <= > >=` and `~` with a tolerance; `none` is null; any failure exits non-zero.
  - `TRI_POSE_LIVE` + `TRI_RAW_CAPTURE` = the live-frame sampler.
- **Verdicts:** "No repro, no fix" (CLAUDE.md). Two AI operators found bugs R1–R5 "confirmed by the state JSON, not by looks weird".
- **Port:** as-is in shape: absolute-first navigation, machine-checkable state, self-asserting scripts. Add block id or hotbar slot and (i, j, k) targets to `tap`. Make sure the new harness is built whenever the game is (the old one was a cargo example that `cargo build --release` skipped, which led to stale binaries).
- **Milestone:** M1b; `bench` M1a; `glide` and `TRI_POSE_RENDER` (milestone video) M1c.

### 19.2 Asserting suites (`viewer/scripts/verify.sh`)
- Rebuilds the harness, regenerates `auto-survey.play` (`gen_survey.py --per 16`, class invariants sampled from `planet_data.npz`), and runs 8 suites: physics-regressions, water-regressions, lake-regressions, flooded-caves, invariant-survey, auto-survey, camera-controls, solar-system.
- Exit codes: 0 green, 1 fail, 2 build failure.
- **Port:** the pattern as-is; re-derive coordinates and expected numbers.
- **Milestone:** M1b.

### 19.3 Reels
All reels exit **0 clean, 3 findings, `--accept` to bless**, and **blessing is a human decision**. Baselines are tracked in `viewer/interchange/{reel,skyreel}/accepted/` and `{perfreel,convreel,convflight}/accepted.json`.

| Reel | Checks | Port | Milestone |
|---|---|---|---|
| `world_reel.py` | A **premise check** first (each pose must contain the class it is named for, B-20), then VOID/GRID lint, then a diff against blessed (mean > 1.0 or > 1% of pixels changed by > 6). About 30 poses (§21) | as-is concept, new poses | M1c |
| `perf_reel.py` | Warm-up `bench 15`, then `bench 30` per scenario; flags avg > +20% or p95 > +35%. **Run alone** | as-is; PLAN's per-milestone number | M1a |
| `sky_reel.py` | 5 poses: sun_zenith, sunset_west, dusk_gradient, moon_zenith, night_stars; disc-presence lint and a moon ROI diff | as-is (the "must not degrade" gate) | M1c |
| `convergence_reel.py` / `convergence_flight.py` | Time domain: does the live view reach the settled one, and how fast (`t_converge`, `tail_ratio`, STUCK). Self-blessing; `convergence_flight.py` replays recorded flights for warm-state bugs (B-16). **Run alone** | rethink for voxel LOD; streaming is still the bug class | M1c / M2 |
| `altitude_ladder.py` | One ground feature at 7 altitudes on one sheet, staircase lint; guards B-17's bar | rethink | M5 |
| `sync_diff.py` | mesh vs voxel, `sea_calib` 0.0% | **drop**; successor: forced-finest vs normal LOD diff | — |

### 19.4 Census and colcensus
- **`examples/census.rs`:** a planet-wide analytic sweep of river transects and lake spokes. It classifies WALL (> 2 m of water above dry ground), JUMP, SEAJUMP and LIP, bisected to 12 m, and outputs teleports plus `--caps` for the bake.
- **`examples/colcensus.rs LAT LON [R]`:** block-truth LIP/EDGE/SHOAL/CAVEP in a disc; exits 2 on LIPs. It sees film and cave water that samplers miss.
- **Port:** rethink for 256-level water: "a wall of resting water" is still the class to catch. A planned `colcensus --survey` over about 100 sites is worth building.
- **Milestone:** M2.

### 19.5 Other instruments
- `pick_accuracy.py`: four-way verification of `pick` (aim round-trip, ray fidelity, on-surface, colour agreement); the template for verifying block picking. M1b.
- `repro_shot.py <shot.png>`: emits the `--capture` command and a `.play` from a sidecar, so any human screenshot becomes a repro. It does not restore `ground_km` **(inferred gap)**. M1b.
- `where.py` / `gen_survey.py`: real coordinates per feature class from `planet_data.npz`. Gotcha: planetgen angles are radians. M1b.
- `tests/golden.rs`: the Rust noise matches `planetgen/noise.py` across 240 values to 1e-9. M1a, if the noise is reused.
- `build_identity.rs` / `TRI_BUILD`: a build stamp over every compiled source plus the toolchain, shown in the title bar, sidecars and MP handshake. M1a.
- Examples: `popdiff` (LOD-pop meter; keep), `tile_cost` and `moonchunkbench` (builder microbenchmarks → a headless chunk bench), `physim` (collision transect), `colof`, `probe`, `mapprobe` (column and generator context), `caveprobe`, `vegprobe`, `lakeprobe`, `seamap`, `colmap` (generator debugging), `moonmap`, `moonfeatures`, `morph_probe` (drop).

### 19.6 `TRI_*` diagnostics
All of them followed one pattern: a standing read-only probe, provably inert when off, production bit-identical. Keep the pattern.
- **Keep:**
  - `TRI_LOD_RING_DEBUG`: false colour per LOD; becomes colour by voxel LOD.
  - `TRI_STREAM_DEBUG`: per-key JSONL lifecycle (missing, admit, start, done, first draw, evict with reason).
  - `TRI_GPU_TIMERS`: six timestamp stamps, non-blocking.
  - `TRI_RAW_CAPTURE`, `TRI_POSE_LIVE`, `TRI_POSE_RENDER`, `TRI_LOG_WALLCLOCK`, `TRI_BUILD`.
- **Drop:** `TRI_NO_MORPH`, `TRI_FORCE_MORPH`, `TRI_NO_BURST`.
- **Rethink:** `TRI_NO_IMPOSTORS`, `TRI_IMPOSTOR_STATS`, `TRI_NO_ROCKS`, `TRI_STREAM`.
- **Moon (keep the pattern):** `TRI_MOON_SKIP_OCTAVE` (an octave bisection that found B-3), `TRI_MOON_LEGACY_FLOORS`.

### 19.7 Process rules worth keeping (`docs/archive/PROCESS.md`)
- Gates proportional to blast radius.
- Cross-model review audits the evidence first.
- **Escaped-gate analysis** on every field bug: which gate should have caught it, and why didn't it?
- Timing gates run alone; pixel gates may overlap.
- Every gate must prove its own premise (B-20, B-25, B-26).

---

## 20. Seam-only machinery: drop on purpose

All of this existed to make a heightfield mesh imitate or meet voxels.

| Item | Where | Keep instead |
|---|---|---|
| Heightfield hole cut, patch lift 1.6 m, rim sink | `shader.wgsl:998-1028`, `1216-1250`; `voxel.rs:1671-1673` | nothing |
| A-5 rim water handoff; deep tiles at 12 octaves | `voxel.rs:659-671`; `terrain.rs:83-86` | nothing |
| Mesh micro-texture twin; per-pixel normal jitter | `shader.wgsl:1459-1474`, `1624-1646` | textures (M3) |
| Per-pixel shorelines (signed shore field) | `shader.wgsl:1251-1290`; `terrain.rs:1311-1590` | maybe for M5 coarse LODs |
| Far-biome categorical reconstruction (8 families) | `shader.wgsl:1297-1418`; `terrain.rs:2663-2780`; `planet.rs:155-190` | per-pixel decisions at coarse LOD (B-19's answer), M5 |
| Karst shader twin | `shader.wgsl:1475-1561` | generate cave mouths in LOD voxels |
| Geomorph, skirts, temporal ease | `terrain.rs:1759-2005`; `renderer.rs:29-44`, `4105-4143` | band-limited LOD plus an arrival fade |
| Tree and rock impostors; candidate cache (64 MiB LRU, region proofs) | `terrain.rs:1632-2392`; `planet.rs:35-54`, `1818-2367` | LOD downsampling that keeps thin features |
| Mesh frost colour; mesh "rocky" slope grey | `terrain.rs:2406-2417`, `2851-2858` | real blocks |
| `wflag`, `beach.w` and RGB-carrier vertex-lane packing | various | real material ids |
| `sync_diff` and the V key; F9 stream levels; the magenta diagnostic | `scripts/sync_diff.py`; `main.rs:1407-1560`, `2235-2253` | an LOD-diff meter |
| `COAST_RANGE_CURVE_*` (12–60 km altitude contour bend) | `shader.wgsl:220-231`, `837-887` | **rethink for M5: these are "Andrew's range-curve knobs", his working irregularity dial (D-18)**; rescale the 28 km wavelength |

---

## 21. Named test sites (lat, lon)

The face frames match, so the old coordinates still name the same places (scaled).

| Site | Coordinates | Note |
|---|---|---|
| Difficulty Lake karst field | 13.346 −4.807 | flagship karst |
| world-reel `river_mouth` (Steikhouth, about 59,000 m³/s) | 7.042 33.477 | also V-1's flagship frame |
| `river_wide` | 15.024 17.662 | |
| `river_valley` | 4.990 −29.403 | |
| approved waterfall | 9.79546 107.60747 | segment 8372 |
| `pond_country` | −0.798 −67.941 | |
| `lake_archi` | 15.4128 −12.570 | |
| `inland_sea` | 20.633 127.615 | |
| `coast_beach` (verified waterline) | 15.104945 150.918258 | |
| `frozen_lake` (thaws at season 0.55) | 69.956 50.980 | |
| `sea_ice` | 83.997 40.220 | |
| `gentle_terrace` | 8.097 −35.673 | B-20 lattice rings |
| `ecotone_corner` | 16.684 −23.367 | |
| `desert` | 0.870 −89.800 | |
| `savanna` | 1.510 40.490 | |
| `jungle` | −0.906 −67.804 | also the B-4c cold-forest pose |
| `peak` (Leleive Range) | 69.0 122.0 | |
| `groves` | 3.960 −32.560 | |
| forest-impostors (conifer / broadleaf / jungle) | 51.851 4.809; 6.974 33.203; 15.942 −78.364 | |
| Andrew's beach-tree pose | 27.902794 33.965275 | |
| D-14 terrace example | 4.377 39.078 | |
| mega-lake wall | 33.539 23.942 | |
| rampart country | 12.199 −44.827 | |
| face-seam ocean split (F-17) | −14.457 −45.0 | |
| moon black hole (B-2) | −35.057 51.770 | moon-local |
| moon seam craters (B-7) | 31.315 44.679; 48.042 −37.405 | moon-local |

**Not a valid reference:** the world reel's `taiga` pose (47.861 14.399) is a treeless lakeshore (B-25).

---

## 22. Open questions for Austin and Andrew

1. **Day length.** 24 minutes (DESIGN) versus 30 (D-5). Do the 7-day month and 84-day year stand?
2. **Strata.** How many stone types, how thick, how strongly folded? Hash-random beds (the old look) or tied to planetgen geology (volcanic, craton, hardness)?
3. **Biome edge widths (D-3 "as-is").** Proportional to the shrink (8 km → about 84 m) or absolute?
4. **Relief.** At 1/70, plains detail is about 1–2 blocks. Does Andrew want a global relief multiplier?
5. **Lake and river sizes.** A softer-than-1/95 width scale for great rivers, and a minimum lake depth in blocks?
6. **Caves.** Shallow and terrain-following (26 blocks), or deep systems? Density by climate and limestone (D-11)?
7. **Snow on local peaks.** Add a virtual lapse rate (closes B-26)?
8. **Leaves.** Solid or passable? Does a closed jungle canopy need higher density?
9. **Distant forests.** Is LOD averaging acceptable in place of the billboards he approved?
10. **Moon.** Tidally locked? N and H? Is r3 still the target?
11. **Lighting.** Is skylight and block-light propagation (Minecraft-style) the cave and tunnel look he wants?
12. **Preservation.** Copy `viewer/interchange/*` notes, reference images and codex findings into a tracked folder?

---

## Andrew's favourites and explicit approvals

- **The 8 cube corners**: "part of the charm" (DESIGN.md).
- **The seamless far-to-near look**: "Existing behavior is very good, colors need a little fine tuning" (D-1).
- **Biome and block colours**: "satisfactory and can be tweaked later when textures are added" (D-15, D-1); biome edge widths "Leave as-is" (D-3).
- **Forests**: "tree billboards at mid distance + vegetation coloring at high"; **biome blending** long within a main block and shorter across blocks (2026-07-09).
- **Biome-warp and coast-range-curve knobs**: labelled as his art dials in code (`planet.rs:65-85`, `shader.wgsl:220-231`); irregularity is his dial (D-18).
- **Shoals wear sand**, not water colour (2026-07-11).
- **Rocks**: boulders collide, alpine erratics go mega (with Austin, 2026-07-16).
- **Karst**: wants cave generation driven by biome (D-11). Breaches are canon (Austin: "karst frigging awesome").
- **Clouds**: realistic-leaning, weather-correlated, multi-layer, faked 3D (D-6). The cyclone spiral-arm density wave, clear eye with dense ring and comma-tail front are his design (WEATHER.md); W-MOTION evolution "All agreed upon".
- **Eclipses**: frequent but "irregular, that's somewhat part of the charm"; the **copper lunar eclipse** "is a nice feature"; solar dimming proportional to the occluded area (D-16).
- **The moon**: his spec drove crater power law, rims and dimples, impact overprinting, one-colour floors, curbed bright rims, rare Tycho-class rays and clustered maria; r3 is his pending target.
- **Time**: the D-5 calendar (lunar-cycle months, 4 seasons, adjustable, an idle-travel idea). "The calendar IS the moon" was accepted.
- **Camera**: C-1 elevation lock, C-2 speed proportional to radius with a far cap, C-3 no auto-pan; freecam with Q/E roll and LMB break / RMB place.
- **Water direction**: waterfalls of up to several dozen blocks with white particles (D-4); seasonal freeze on revisit (D-9); both ice cliffs and tapered glaciers (D-13); meander extent scaling with flow (D-12, agreeing with Austin's bank proposal); small or slanted terraces fine, no big lake cliffs on smooth terrain (D-14); realistic overcast (D-7); crevice-biased rain (D-8).
- **Austin's** (not Andrew's), for the record:
  - strata named as a must-keep;
  - ground patchiness approved (L-1(c));
  - the torch night shot called "the game's first cozy screenshot";
  - "we fly at medium altitude a lot";
  - caves near water should be full of water;
  - weather gradations must be continuous.

## Problems that should not come back

- **Water walls and perched water**: lakes filling a footprint instead of terrain (F-1), levels anchored to beds (F-4), pond ramparts and walls (A-4/A-5, F-22), dry pits below a river surface (F-23), rivers riding above the terrain (Andrew's note). Generated water must be at rest, at grade, and never perched (W1–W3).
- **Sea by elevation alone**: navy lagoons and flooded dry basins; an ocean mask that disagrees across cube-face seams (F-17).
- **Big cliffs where terrain is smooth**: kilometre coastal cliffs from roughness spikes; mega-lake dirt steps (D-14 / W-7); 600 m summit ice cliffs from merged depression chains (W-5b, S-3).
- **Shading bands**: fall-line stripes and terrace rings from quantised normals (F-2, F-5); near-black lee faces (F-8); riser smears (F-11); bright/black water steps (F-9); chunk-grid patterns from position-derived normals; a set sun still lighting canopies; a low moon striping terraces.
- **f32 and hash artefacts**: crawling or banded dither from planet-magnitude coordinates or `fract()` hashes (F-25, V-10); a 12 km grid from a cell-constant raster sampler; far bodies jittering per tile.
- **Raster artefacts**: visible texels and noise-octave seams (Andrew's "texel artifacts"); a 255 ocean sentinel painting blue plates on desert coasts.
- **LOD category staircases**: biome, shore and snow edges quantised to the LOD lattice with regular beading (B-19 / D-18); borders that show at range and vanish up close (L-1); LOD-ring tint and density seams (B-18, B-1); detail rings popping (V-8, V-6).
- **Vegetation**: orchard rows from a linear hash; trees on beaches and razor-straight vegetation lines (Andrew's note 3); trees riding edits; trees disagreeing across face edges; far stand-ins placed where no block tree exists (E-2); rock "carpets".
- **Streaming that never converges or hitches**: synchronous builds on ascent (B-4); cold forest builds of 3 s (B-4c); evicting the wanted set (B-21); eviction storms (B-10); sequential fallback builds (F-24); LOD distance measured to the sea-level sphere (B-23); superlinear patch cost (B-24). Andrew flagged "lagspikes when ascending" as IMPORTANT.
- **Camera and physics**: −90° pitch spinning the view (B-5); a stale underwater flag (R2/R4); frozen lakes that swim (R3, F-18); a hole you can't step out of (R5-3); walls that trap you (the deadlock); NaN poisoning and corrupt saves (F-19); wall-clock time breaking determinism (F-20); photo restores that don't reproduce the photo (F-21).
- **Weather**: unbounded shear shredding clouds; speckled orbital decks with uncleared lanes (B-8); camera-anchored weather pulsing the planet from orbit; the snowline receding as you climb; snow dusting on water and ice (V-9); stars and the sun showing through clouds.
- **The moon**: unbounded crater pits (B-2); rim cliffs from a step ejecta term (B-3); overlapping maria darkening; snake rays and banded rays (r3); circular halos on oblong craters (B-6); craters changing across cube seams (B-7); vertically exaggerated moon blocks; phantom eclipses from rotating about the observer; moon photos filed under Neisor.
- **Gates that guard nothing**: blessed poses that don't contain their class (B-20, B-25, B-26); perf probes on barren poses or warmed before measuring (B-4a); timing gates run beside other renders; a stale harness binary verified by mistake.
