# M1a: engine skeleton (Fable, 2026-10-25)

Branch `worktree-agent-a79bc8ba92cdccc0c`, crate `game/`. Evidence in
`docs/missions/M1a-evidence/` (downscaled) and, full size, under
`C:\code\triangulum\.claude\worktrees\agent-a79bc8ba92cdccc0c\game\out\`
(gitignored, lives only on this machine).

## What was built

`game/` is a standalone crate (not in the viewer workspace) grown from
`spike/voxel-core`:

| module | what |
|---|---|
| `cubesphere_stub` | the `crates/cubesphere` API (FACES, Projection, warp, unwarp, face_to_dir, dir_to_face). Equiangular only; the others panic. Every lattice <-> direction conversion in the engine goes through it. |
| `grid` | `Planet` (block size s, world height H, sea level, N, R = s N sqrt(6/4 pi)), `WorldConfig` from `assets/world.json` with CLI overrides, `ChunkKey`/`FootprintKey`/`BlockPos`, a fast hasher for the per-frame chunk maps. Asserts H is a power of two and N % H == 0. |
| `world` | block registry from `assets/blocks.json` (stable u16 ids, linear colours, solid/liquid), palette-compressed 32^3 chunks (bit-packed indices, uniform fast path, canonical bytes), a water-level byte layer allocated only when a chunk holds water, `ChunkStore`. |
| `worldgen` | generator v1, a pure function of (seed, position, voxel size). `bake.rs`: the `Bake` trait with one `RasterBake` reader for both the new equal-angle world bake (meta.json, only the needed layers read through the byte offsets) and the old gnomonic viewer bake. Catmull-Rom for elevation/temperature/precipitation, bilinear for masks. `noise.rs`: seeded Perlin gradient noise and fBm. `strata.rs`: beds as real blocks. |
| `mesh` | binary greedy meshing into 8-byte quads, per-vertex AO (merges only across equal AO), separate water quads, quads split at curvature-grid cells. |
| `stream` | an octree of cubic chunks per face chosen by 3D distance (factor 5 screen-size policy from the prototype), chunks outside their footprint's terrain band never visited, horizon culling, rayon jobs, ancestor/descendant fallbacks, draw lists (opaque front-to-back, water back-to-front). |
| `render` | wgpu 30: MDI vertex pulling, colours from the registry, AO, translucent water pass (premultiplied blend, no depth write, both faces), sky gradient with a sun disc, distance fog, timestamp queries, PNG readback. |
| `app` | window, free-fly camera (WASD, mouse look on click/Esc, Space/Ctrl, scroll 1 m/s..5 km/s, Shift boost, P presets, V vsync, F water, H hud, R to ground), `--at LAT LON ALT` (`--look HEADING PITCH`), egui 0.35 HUD (fps, 1 % low over 5 s, GPU/CPU ms, chunks per LOD drawn and loaded, lat/lon/alt, crosshair block lattice coords and name), `--screenshot PATH --exit-after S`. |
| `bench` | `--bench`: the seven static poses, the 60 s flight, AO cost, worker costs, `results.md` + PNGs. `--shots`: Andrew's vertical-scale renders with marker columns and captions. |
| `ui` | the HUD and captions, painted through a small wgpu-30 egui painter (egui-wgpu 0.35 pins wgpu 29). |

### Generator v1

- Height = bake elevation x vertical scale (default R / 6,371 km = 1/70.3)
  plus gradient-noise relief: eight octaves from 10 km to 16 m (the bake has
  no structure between its ~30 km cells and its texels), first-octave
  amplitude 0.5 x the bake's roughness field x the dial, gain 0.42, each
  octave's Earth-scale slope capped at 0.7 (otherwise the mountains' 5 km
  per-cell roughness extrapolates into 1 m cliffs everywhere), octaves finer
  than four voxels faded out so every LOD samples one band-limited surface.
  Relief fades to a quarter near the shoreline and is weaker on the sea floor.
- Ocean: ocean_frac > 0.5 and elevation < 0, water to sea level (255 in the
  water-level layer), ice on the top voxel below -4 C.
- Materials with dithered thresholds (no texel patches): snow below -3 C,
  stone on slopes steeper than ~0.95, beach sand within ~2.5 m of sea level
  near the ocean mask, desert sand over real sandstone where precipitation is
  under 130 mm and it is warm, dry grass under 320 mm, grass otherwise; sea
  floor sand, gravel below 6 m. Soil 1-4 blocks from planetgen's soil_km
  (thinner on slopes).
- Strata: under the soil, beds where bed = floor((z + fold) / thickness).
  Pairs of 8 blocks are split by a hashed point into two beds of 2-6 blocks;
  the fold is 18 m at 2 km plus 6 m at 620 m plus a 0.6 m block-scale warp,
  all unscaled. Palette = the feature inventory's linear tints (shale,
  mudstone, umber, sandstone, limestone) at full strength; edges hard.
  `GeoHint` (craton, volcanic, hardness from erodibility) is read from the
  new bake and routed into `rock_at`, which already has the basalt/basement
  branch for later.
- Markers (debug): bright 100 m (4x4) and 2 m (1x1) columns; a `below`
  marker stamps air and cuts a quarry pit.

### LOD and world height

Chunks are 32^3 at voxel sizes 1, 2, 4 ... blocks. The coarsest LOD in use
is min(log2 H, trailing_zeros(sea level)): a voxel grid can only put sea
level on a voxel boundary when the voxel divides the sea level, and one
level coarser the whole ocean (or the whole planet) is either a voxel too
high or missing. With H 2048 and sea level 512 that is LOD 9 (512 m), the
prototype's cap; with H 512 / sea 256 it is LOD 8; with H 4096 / sea 512
still 9. H only buys coarser LODs if the sea level rises with it.

## Evidence

### Tests (`cargo test --release`, 21 passed)

Stub round trips (warp/unwarp, face/dir, shared edges, equal angles along
an edge, the edge-column snap); grid (equal-area radius, N % H, lattice and
lat/lon round trips, border columns snap onto the neighbour face's lattice,
chunk counts); LOD alignment (a coarse voxel covers exactly 2^L fine
columns, eight children tile the parent); palette round trips (3-bit and
10-bit indices straddling words, uniform fast path, water layer); gradient
noise bounds; strata bed thickness 2-6 and fold bounds; generator
determinism (identical chunk bytes across two runs and across rayon pools of
1 and 8 threads); the new bake agrees with the old one and samples smoothly.

### Bench (PROVISIONAL: other agents used the CPU; this machine was not idle)

Machine: RTX 2060, Ryzen 7 2700X, 16 rayon threads, 1920x1080 offscreen,
Vulkan. Pose numbers are 120 frames after streaming settled; the flight is
60 s wall clock at 50 m AGL and 50 m/s over the hills anchor, climbing to
5 km over 15 s and diving back. 1 % low = 1000 / mean of the worst 1 % of
frame times. Tables below are copied from each run's `results.md`.

Flight summary (the project number; all provisional):

| run | frames | avg fps | 1 % low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|---|
| H 2048, sea level 512, s 1 m, N 131072 (max LOD 9) | 34800 | 583.5 | 266.6 | 7.48 | 1.65 / 3.28 | 1.36 / 2.77 | 0.17 / 0.19 |
| H 512, sea level 256, s 1 m, N 131072 (max LOD 8) | 37703 | 632.1 | 312.2 | 6.66 | 1.56 / 2.84 | 1.26 / 2.41 | 0.17 / 0.18 |
| H 4096, sea level 512, s 1 m, N 131072 (max LOD 9) | 58843 | 985.1 | 357.2 | 9.21 | 1.03 / 2.48 | 0.69 / 2.10 | 0.17 / 0.19 |
| s 0.5 m, N 262144, H 2048, sea level 512 (max LOD 9) | 34070 | 570.6 | 268.2 | 6.06 | 1.61 / 3.35 | 1.41 / 2.90 | 0.18 / 0.20 |

World height comparison: the static poses at H 2048 and H 4096 load and
draw exactly the same chunks (3409 resident at the standing pose, the same
per-LOD counts at every pose) and cost the same; a chunk fully above or
below the terrain band is never visited, so H changes nothing until the
coarsest LOD changes. H 512 with sea level 256 caps the coarsest LOD at 8
(256 m voxels), which is why its 500 km pose draws 1032 chunks instead of
295; that is still 0.11 ms of GPU. The flight averages differ between the
runs (583 / 632 / 985 fps) far more than the static poses do, and the
H 4096 run was the only one with nothing else on the machine, so read the
differences as load noise, not as H. Conclusion: a taller world costs
nothing with cubic chunks, as claimed, provided the sea level stays a
multiple of the coarsest voxel wanted.

Half-metre blocks (s 0.5, N 262144): 4297 resident chunks at the standing
pose against 3409, 2.1 ms CPU against 1.7 in the same-load run, flight
571 fps average, 268 fps 1 % low, 6.1 ms worst frame.

#### H 2048, sea level 512, s 1 m, N 131072 (max LOD 9)

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.12 | 0.12 | 1.67 | 2.25 | 0.18M | 0.00M | 814 | L0:125 L1:96 L2:89 L3:87 L4:85 L5:87 L6:91 L7:73 L8:51 L9:10 | 3409 | 4.8 | 1.2 |
| stand_mountains | 0.16 | 0.16 | 1.95 | 2.60 | 0.19M | 0.00M | 939 | L0:124 L1:92 L2:85 L3:88 L4:91 L5:90 L6:85 L7:88 L8:59 L9:8 | 3769 | 4.9 | 0.8 |
| alt_100m | 0.19 | 0.19 | 1.79 | 2.38 | 0.18M | 0.00M | 816 | L0:116 L1:100 L2:89 L3:87 L4:84 L5:88 L6:91 L7:74 L8:53 L9:14 | 3356 | 4.7 | 0.7 |
| alt_1km | 0.18 | 0.18 | 1.18 | 1.92 | 0.14M | 0.00M | 582 | L2:47 L3:103 L4:84 L5:91 L6:87 L7:72 L8:62 L9:15 | 2183 | 3.6 | 0.1 |
| alt_10km | 0.16 | 0.16 | 0.42 | 0.63 | 0.10M | 0.00M | 362 | L5:32 L6:108 L7:84 L8:92 L9:30 | 1088 | 2.0 | 0.1 |
| alt_100km | 0.10 | 0.10 | 0.26 | 0.65 | 0.03M | 0.00M | 146 | L8:13 L9:133 | 258 | 0.4 | 0.1 |
| alt_500km | 0.08 | 0.08 | 0.41 | 0.65 | 0.05M | 0.00M | 295 | L9:295 | 295 | 0.4 | 0.0 |

Fixed flight:

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 34800 | 583.5 | 266.6 | 7.48 | 1.65 / 3.28 | 1.36 / 2.77 | 0.17 / 0.19 |

Worker costs (median / p95 microseconds per chunk):

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 6957 | 2130 | 3609 | 239 | 319 |
| 1 | 3494 | 2719 | 3637 | 224 | 315 |
| 2 | 2280 | 2743 | 3515 | 239 | 301 |
| 3 | 2127 | 2475 | 3136 | 229 | 273 |
| 4 | 2149 | 2337 | 3689 | 223 | 395 |
| 5 | 1436 | 2839 | 3494 | 275 | 366 |
| 6 | 1175 | 2457 | 3203 | 263 | 345 |
| 7 | 852 | 2312 | 3818 | 278 | 357 |
| 8 | 637 | 2154 | 3148 | 272 | 326 |
| 9 | 350 | 2239 | 3908 | 261 | 823 |

#### H 512, sea level 256, s 1 m, N 131072 (max LOD 8)

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.12 | 0.12 | 1.41 | 1.91 | 0.17M | 0.00M | 802 | L0:125 L1:96 L2:89 L3:87 L4:85 L5:87 L6:91 L7:69 L8:53 | 3260 | 4.6 | 1.1 |
| stand_mountains | 0.13 | 0.13 | 2.14 | 2.73 | 0.18M | 0.00M | 928 | L0:124 L1:92 L2:85 L3:88 L4:86 L5:90 L6:85 L7:88 L8:61 | 3692 | 4.7 | 0.8 |
| alt_100m | 0.19 | 0.19 | 1.60 | 2.34 | 0.17M | 0.00M | 799 | L0:116 L1:100 L2:89 L3:87 L4:85 L5:87 L6:91 L7:70 L8:54 | 3205 | 4.5 | 0.7 |
| alt_1km | 0.18 | 0.18 | 1.79 | 2.57 | 0.12M | 0.00M | 555 | L2:47 L3:95 L4:88 L5:89 L6:83 L7:72 L8:63 | 2018 | 3.3 | 0.1 |
| alt_10km | 0.15 | 0.15 | 0.93 | 1.28 | 0.08M | 0.00M | 349 | L5:33 L6:106 L7:84 L8:107 | 1076 | 1.8 | 0.1 |
| alt_100km | 0.11 | 0.11 | 0.71 | 1.08 | 0.07M | 0.00M | 443 | L8:433 | 801 | 1.0 | 0.1 |
| alt_500km | 0.11 | 0.11 | 0.78 | 1.08 | 0.13M | 0.00M | 1047 | L8:1032 | 1032 | 1.3 | 0.1 |

Fixed flight:

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 37703 | 632.1 | 312.2 | 6.66 | 1.56 / 2.84 | 1.26 / 2.41 | 0.17 / 0.18 |

Worker costs (median / p95 microseconds per chunk):

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 6957 | 2107 | 3634 | 237 | 303 |
| 1 | 3494 | 2595 | 3572 | 209 | 322 |
| 2 | 2280 | 2738 | 3468 | 238 | 307 |
| 3 | 2119 | 2481 | 3336 | 228 | 280 |
| 4 | 1708 | 2943 | 3597 | 274 | 349 |
| 5 | 1396 | 2479 | 3102 | 254 | 326 |
| 6 | 1168 | 2215 | 2841 | 250 | 332 |
| 7 | 850 | 2210 | 3048 | 263 | 323 |
| 8 | 1201 | 2074 | 3117 | 253 | 341 |

#### H 4096, sea level 512, s 1 m, N 131072 (max LOD 9)

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.12 | 0.12 | 0.77 | 0.91 | 0.18M | 0.00M | 814 | L0:125 L1:96 L2:89 L3:87 L4:85 L5:87 L6:91 L7:73 L8:51 L9:10 | 3409 | 4.8 | 1.1 |
| stand_mountains | 0.13 | 0.13 | 0.92 | 1.10 | 0.19M | 0.00M | 939 | L0:124 L1:92 L2:85 L3:88 L4:91 L5:90 L6:85 L7:88 L8:59 L9:8 | 3769 | 4.9 | 0.7 |
| alt_100m | 0.19 | 0.19 | 0.89 | 1.91 | 0.18M | 0.00M | 816 | L0:116 L1:100 L2:89 L3:87 L4:84 L5:88 L6:91 L7:74 L8:53 L9:14 | 3356 | 4.7 | 0.7 |
| alt_1km | 0.18 | 0.18 | 0.62 | 0.71 | 0.14M | 0.00M | 582 | L2:47 L3:103 L4:84 L5:91 L6:87 L7:72 L8:62 L9:15 | 2183 | 3.6 | 0.1 |
| alt_10km | 0.16 | 0.16 | 0.37 | 0.42 | 0.10M | 0.00M | 362 | L5:32 L6:108 L7:84 L8:92 L9:30 | 1088 | 2.0 | 0.1 |
| alt_100km | 0.10 | 0.10 | 0.19 | 0.21 | 0.03M | 0.00M | 146 | L8:13 L9:133 | 258 | 0.4 | 0.0 |
| alt_500km | 0.08 | 0.08 | 0.20 | 0.22 | 0.05M | 0.00M | 295 | L9:295 | 295 | 0.4 | 0.0 |

Fixed flight:

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 58843 | 985.1 | 357.2 | 9.21 | 1.03 / 2.48 | 0.69 / 2.10 | 0.17 / 0.19 |

Worker costs (median / p95 microseconds per chunk):

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 6957 | 2084 | 3480 | 234 | 295 |
| 1 | 3494 | 2589 | 3420 | 214 | 307 |
| 2 | 2280 | 2736 | 3472 | 240 | 300 |
| 3 | 2127 | 2464 | 3119 | 228 | 277 |
| 4 | 2149 | 2337 | 3705 | 224 | 394 |
| 5 | 1436 | 2843 | 3722 | 277 | 360 |
| 6 | 1175 | 2449 | 3602 | 262 | 355 |
| 7 | 852 | 2379 | 3193 | 279 | 334 |
| 8 | 637 | 2170 | 3032 | 269 | 325 |
| 9 | 350 | 2175 | 3490 | 258 | 433 |

#### s 0.5 m, N 262144, H 2048, sea level 512 (max LOD 9)

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.13 | 0.13 | 2.08 | 2.61 | 0.21M | 0.00M | 894 | L0:129 L1:88 L2:96 L3:89 L4:87 L5:85 L6:87 L7:91 L8:69 L9:53 | 4297 | 5.3 | 1.6 |
| stand_mountains | 0.16 | 0.16 | 1.83 | 2.75 | 0.20M | 0.00M | 1046 | L0:157 L1:85 L2:92 L3:85 L4:88 L5:86 L6:90 L7:85 L8:88 L9:61 | 4923 | 5.4 | 1.0 |
| alt_100m | 0.20 | 0.20 | 1.73 | 2.48 | 0.19M | 0.00M | 826 | L0:47 L1:96 L2:100 L3:89 L4:87 L5:85 L6:87 L7:91 L8:70 L9:54 | 3916 | 5.2 | 0.8 |
| alt_1km | 0.18 | 0.18 | 1.22 | 1.73 | 0.14M | 0.00M | 555 | L3:47 L4:95 L5:88 L6:89 L7:83 L8:72 L9:63 | 2014 | 3.6 | 0.1 |
| alt_10km | 0.16 | 0.16 | 0.97 | 1.40 | 0.10M | 0.00M | 349 | L6:33 L7:106 L8:84 L9:107 | 1076 | 2.1 | 0.1 |
| alt_100km | 0.12 | 0.12 | 0.47 | 0.90 | 0.08M | 0.00M | 443 | L9:433 | 801 | 1.1 | 0.1 |
| alt_500km | 0.12 | 0.12 | 0.50 | 1.02 | 0.15M | 0.00M | 1047 | L9:1032 | 1032 | 1.4 | 0.0 |

Fixed flight:

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 34070 | 570.6 | 268.2 | 6.06 | 1.61 / 3.35 | 1.41 / 2.90 | 0.18 / 0.20 |

Worker costs (median / p95 microseconds per chunk):

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 12470 | 2003 | 3131 | 175 | 281 |
| 1 | 6928 | 2038 | 3324 | 195 | 285 |
| 2 | 3247 | 2706 | 3556 | 220 | 313 |
| 3 | 2280 | 2736 | 3420 | 239 | 298 |
| 4 | 2107 | 2482 | 3043 | 227 | 273 |
| 5 | 1708 | 2932 | 3450 | 271 | 353 |
| 6 | 1396 | 2521 | 2977 | 252 | 334 |
| 7 | 1168 | 2236 | 2774 | 250 | 334 |
| 8 | 850 | 2223 | 2910 | 266 | 324 |
| 9 | 1201 | 2070 | 2774 | 249 | 312 |


### AO triangle cost

Re-meshing every resident chunk at the standing pose with and without AO (CPU only, same geometry):

- H 2048, sea level 512, s 1 m, N 131072 (max LOD 9): over 3409 loaded chunks: 300824 opaque quads with AO vs 258970 without (+16.2%), i.e. 0.60M vs 0.52M triangles
- H 512, sea level 256, s 1 m, N 131072 (max LOD 8): over 3260 loaded chunks: 281304 opaque quads with AO vs 240114 without (+17.2%), i.e. 0.56M vs 0.48M triangles
- H 4096, sea level 512, s 1 m, N 131072 (max LOD 9): over 3409 loaded chunks: 300824 opaque quads with AO vs 258970 without (+16.2%), i.e. 0.60M vs 0.52M triangles
- s 0.5 m, N 262144, H 2048, sea level 512 (max LOD 9): over 4297 loaded chunks: 348311 opaque quads with AO vs 299560 without (+16.3%), i.e. 0.70M vs 0.60M triangles

AO quads only merge across equal corner values, so cliffs and step edges fragment; flat tops stay one quad. The GPU cost of the extra 16-17 % triangles is invisible at 0.1-0.2 ms per frame.

### Water

Water is translucent (premultiplied alpha 0.62 with a Fresnel-ish ramp and
a sun glint), drawn after the opaque pass with depth test but no depth
write, both faces, sorted back to front. Cost in the coast view (`docs/missions/M1a-evidence/hud_coast_water.png`, 400 m over the steep coast, water filling half the frame): 0.01 M water triangles on top of 0.26 M opaque, 0.30 ms GPU against 0.20 ms over land at the same altitude; 1021 draws. Translucent is affordable and is what ships.

### Andrew's vertical-scale renders

Six places x four configurations: 1/70 and 1/20 at H 2048 / sea 512, 1/8 at
H 2048 / sea 1024 (trench clipped at bedrock, caption says so) and 1/8 at
H 4096 / sea 1536. Contact sheets (full size, 2904x1688):

- `C:/code/triangulum/.claude/worktrees/agent-a79bc8ba92cdccc0c/game/out/vs/sheet_a_range_ground.png`
- `C:/code/triangulum/.claude/worktrees/agent-a79bc8ba92cdccc0c/game/out/vs/sheet_b_range_1km.png`
- `C:/code/triangulum/.claude/worktrees/agent-a79bc8ba92cdccc0c/game/out/vs/sheet_c_range_silhouette.png`
- `C:/code/triangulum/.claude/worktrees/agent-a79bc8ba92cdccc0c/game/out/vs/sheet_d_trench.png`
- `C:/code/triangulum/.claude/worktrees/agent-a79bc8ba92cdccc0c/game/out/vs/sheet_e_coast_shelf.png`
- `C:/code/triangulum/.claude/worktrees/agent-a79bc8ba92cdccc0c/game/out/vs/sheet_f_strata_cut.png`

The single renders are under `C:/code/triangulum/.claude/worktrees/agent-a79bc8ba92cdccc0c/game/out/vs/{s70_h2048,s20_h2048,s8_h2048,s8_h4096}/`.

Readings from the sheets:
- At 1/70 the planet is flat at human scale: the highest range is 121 m, so
  from 4 km away it is a horizon line and from 1 km up a field of 1-block
  contour steps. Nothing reads as a mountain. The trench (-157 m, tens of
  km wide) is invisible from anywhere.
- At 1/20 the range is 428 m; from the ground it is still a slope of
  1-block steps, from 1 km up it reads as rolling relief.
- The trench is a slot one planetgen cell wide that the bake's 30 km
  smoothing turns into a modest valley: at 1/8 with H 2048 (floor clipped
  at -1024 m) it reads as a valley between hills; at 1/8 with H 4096 from
  8 km it is a crease at the foot of the far wall. Nothing at 1/70 or 1/20
  reads as a trench.
- At 1/8 the range is 1072 m and the coast shot finally looks like a coast:
  a shelf, a steep slope and a 667 m basin (`sheet_e_coast_shelf.png`,
  bottom right). The ground shot shows a 20 % slope as a staircase of 1 m
  risers seen uphill from 1.7 m; that is the honest look of 1 m voxels on
  such a slope, not a bug (the generator profile there has 0.23 m steps,
  verified with `--probe`).
- The horizon on a 90.6 km planet is 950 m from eye height and 5.6 km to a
  121 m peak, so "a silhouette of the whole range against the sky from near
  sea level" is geometrically impossible at 1/70 and only partly possible at
  1/8 (12 km to a 1072 m peak). The (c) shots are the best the geometry
  allows.
- The relief dial at its default did not look broken once the slope cap was
  in; before the cap, 1/8 and 1/20 mountains were a thicket of metre cliffs.
- Fog: the distance fog starts from 30 km (plus three times the altitude)
  for these renders; the prototype's 9 km range hid the 8-12 km trench and
  silhouette subjects entirely. The same range ships in the app for now.

### Strata

`docs/missions/M1a-evidence/strata_cut_1_70.png` and
`docs/missions/M1a-evidence/sheet_f_strata_cut.png`: a 48 m quarry pit on the hills, 28 m deep, seen
from its floor. Grass and dirt over 2-6 block beds in the five tints, hard
edges, the fold visible as stepped bed boundaries across the wall.

### Windowed app

`docs/missions/M1a-evidence/hud_app_peak.png`: the HUD in the windowed app
at 200 m over the highest range (vsync off): 393 fps, 1 % low 242, 3.2 ms
frame, 1.9 ms CPU, 0.20 ms GPU, chunks per LOD, lat/lon/alt, crosshair
block `face 4 i 94883 j 82914 k 598 snow`.

## Not verified

- Timings on an idle machine. Everything above is provisional.
- The windowed app was only driven headlessly (`--exit-after`); nobody has
  flown it with a mouse yet. Mouse capture uses winit Confined mode.
- Cube-edge seams: the equiangular stub lets border columns snap onto the
  neighbour face's lattice (tested) and the relief noise is 3D, so there
  should be none; I did not render a cube edge.
- Vertical LOD transitions inside one column (a fine chunk under a coarse
  one) are not skirted; a hole seen from underneath is possible there.
- The s = 0.5 m run only proves the parameters work and reports cost; no
  one has judged the look.
- The water layer is filled with 255 and never read back by anything yet.

## What I would change in the plan

1. Sea level and H are coupled through the coarsest LOD: pick sea level as
   a power of two and let H be the smallest power of two that fits the
   terrain; H above that is free but buys nothing until the sea level rises.
2. The vertical-scale question is really a voxel-size question: at 1/70, 1 m
   blocks show no mountains at all; at 1/8 they show cliffs and 1 m
   staircases on every slope. Half-metre blocks at 1/20 is the combination
   the sheets suggest trying next.
3. The 1-10 km band of relief is the engine's to invent; the bake cannot
   supply it. That band decides how mountains look more than the vertical
   scale does, and it deserves a real spectrum (ridged noise in orogens,
   erosion-shaped valleys) rather than fBm.
4. The main-thread per-frame rebuild (desired set, render set, draw lists)
   is 1-2 ms at factor 5 and 4 ms at 1/8 where 13k chunks are resident. It
   is the first thing that breaks, as the prototype said; M1c's "main-thread
   fixes" should be incremental traversal and cached culling.
5. `crates/cubesphere`: `warp` need only be defined on the face; the engine
   extrapolates past edges in gnomonic space and snaps to the neighbour's
   lattice, so the swap should be mechanical.

## Feedback

- The bench's `--shots` tool, the contact-sheet script and `--probe` were
  built here. The probe (height profile along 96 columns) is what separated
  a generator problem from a rendering one in minutes; keep it.
- The new world bake's meta.json byte offsets made the reader a one-hour
  job. The one surprise was the equal-angle layout: texels are uniform in
  the lattice parameter, so the old reader's texel mapping had to go through
  atan; both layouts now share one struct.
- egui offscreen captures need animations off and two layout passes; the
  fade-in otherwise leaves captions half transparent in a way that depends
  on wall-clock time between captures. Worth a line in the viewer notes if
  anyone captures egui offscreen again.
- Timing runs and builds cannot share the machine: cargo cannot replace a
  running binary, and PIL work during a run inflates the static-pose CPU
  numbers (the first run's stand_hills CPU is 1.7 ms against 0.9 ms when
  nothing else ran).
