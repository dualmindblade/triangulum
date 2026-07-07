# triangulum-viewer — Phase 0

Fly over Neisor, from orbit down to ~100 m, rendered from the planet dataset
on the gnomonic cube-sphere. This is the proof of the architecture the game
will use: one quadtree per cube face drives both procedural content and LOD.

## Run

```
# once: bake the planet onto cube-face rasters (from the repo root)
python scripts/bake_faces.py output/seed42_r8 1024
# once: export river courses + lakes from the drainage graph
python scripts/bake_rivers.py output/seed42_r8

# interactive
cd viewer
cargo run --release

# headless screenshot (no window)
cargo run --release -- --capture shot.png --lat 22 --lon 28 --alt 500 --pitch -30 --yaw 90
```

## Controls

| input | action |
|---|---|
| **click** | capture the mouse for raw free-look (Esc releases; Esc again quits) |
| drag | free look when the mouse isn't captured |
| W A S D | move along your heading (fly: speed scales with altitude; Shift = sprint) |
| scroll | altitude (fly mode; above 100 km the view auto-tilts with altitude, below that pitch is all yours). Descends to ~2.5 m — hover over the grass, sink into cave pits (roofs are solid) |
| **G** | walk mode — real gravity: pressed in flight you skydive from there. Walls stop you, one-block ledges step up, cave ceilings bump your head, pits drop you in. Water: you sink slowly; hold Space to swim up |
| **Space** | jump (walk mode, when standing) / swim up (in water) |
| **Q / E** | break a block / place a block against the face you're aiming at (edits persist to `assets/edits_seed*.bin`) |
| **R** | place/remove a torch on the block you're aiming at — real light at night and in caves (persists to `assets/torches_seed*.bin`) |
| **F** | back to fly mode |
| **T** | teleport: type `lat lon [alt km]` into the title bar, Enter to go |
| **P** | screenshot to `interchange/shot_lat…_lon…_alt…km_….png` (coordinates in the filename) |
| Esc | quit (or cancel the teleport prompt) |

Scenic destination: the planet's highest peak (8.58 km) is at
`--lat 69 --lon 122`.

**`--patch N`** (0.3–2.0, default 1) scales the voxel-patch disc — at 2 the
blocky world reaches ~1 km around you. Chunks stream in on background
threads (mesh terrain shows until they land, then blocks rise through the
feathered rim), so bigger patches cost VRAM (~1.5 GB budget, LRU-evicted)
and build throughput, not frame hitches.

The window title shows mode + coordinates. A **day/night cycle** runs by
default: the sun hangs in space while the planet turns (20-minute day,
`--day-len N` seconds to change it), starting mid-morning where you spawn.
Sunset brings out the stars; torches matter after dark. `--day-len 0`
restores the old always-noon-where-you-are mode, and `--sun-lat/--sun-lon`
pins the sun exactly (screenshots stay reproducible). `--exagg N` scales
terrain height (default 10; walk mode is best at `--exagg 1`).

## What's here (and what it proves)

| file | what |
|---|---|
| `src/planet.rs` | face rasters + gnomonic cube-sphere math (the game's coordinate system) |
| `src/terrain.rs` | the LOD quadtree: screen-space-error tile selection, tile meshing with skirts |
| `src/camera.rs` | orbital camera in f64 (planet-scale precision) |
| `src/renderer.rs` | wgpu pipeline, per-tile buffers + cache, offscreen capture |
| `src/shader.wgsl` | camera-relative vertex transform, sun lighting, haze |

Key invariants proven end-to-end:
* every tile is generated independently from (face, level, ix, iy) — the
  "query the hierarchy at any depth" property the whole game design rests on
* vertices live relative to their tile origin; only tile-origin-minus-camera
  (computed in f64) reaches the GPU — no float jitter at 8,660 km scale
* the planet dataset is the top of the LOD pyramid; where its resolution runs
  out (~level 5) is exactly where Phase 1's noise octaves will take over

## Phase 1 (done)

* `src/noise.rs` — exact port of `planetgen/noise.py`; `cargo test` proves
  240 golden values match the Python original to 1e-9 (same seed = same
  planet, cross-language).
* Band-limited detail octaves below the raster floor: every quadtree level
  adds exactly the octaves its vertex spacing can carry (`fbm_band`,
  `ridged_band`), enveloped by the planet map's elevation. MAX_LEVEL 12
  (~100 m vertex spacing).
* Seam fixes: edge-inclusive rasters (rebake required!), ghost-ring normals,
  culling off for skirts. rayon-parallel tile builds.

## Phase 2 (done)

* **Voxels are real**: below 2.5 km altitude, terrain near the camera meshes
  as quantized 1 m columns on the 10,000,000-column face lattice (`src/
  voxel.rs`) — the diamond prisms from the game spec, chunked 32x32, built in
  parallel, cached. The heightfield is *cut away* under the voxel patch
  (fragment discard), so every pixel belongs to exactly one system.
* Chunk rings cross cube-face edges seamlessly; Q/E break/place blocks.

## Phase 3 (landscape generation — done)

The planet map now drives a full procedural landscape (rebake required:
`python scripts/bake_faces.py output/seed42_r8 1024` — face_*.bin carries
roughness, precipitation, temperature, and river-flow layers now):

* biome-real materials: grass tinted per Köppen class, deserts sand over
  stone, snow/ice by temperature, cliffs bare rock, beaches, dirt strata
* relief follows the map's roughness metric (jagged ranges, calm plains)
* rivers carved as gorges with painted/blocky water, ponds in wet lowlands
* hash-placed full-block trees: jungle canopies, conifers, broadleaf
  groves, acacias, tundra shrubs — density/species by biome and treeline
* cave tubes under dry land; their mouths are real pits you can fall in
* water is wadeable: walk in and you sink to the floor, the view tints
  blue while your eyes are under; distant forests darken the far mesh

## Phase 5 (rivers & lakes from the map — done)

Rivers are no longer noise: courses come from the planetgen drainage graph
(`scripts/bake_rivers.py` -> `assets/rivers.bin`, ~20k cell->receiver
segments smoothed and shipped with flow + monotonic downstream water
levels). The generator measures exact distance to the nearest segment:
width/depth follow hydraulic geometry (~3 sqrt(Q)), fine relief flattens
into a floodplain near the channel, noise only adds bounded meanders, and
headwaters taper in from nothing. Lakes fill to their spill level inside
the Voronoi footprint of their map cells — the big ones are inland seas
with real shorelines and islands. Rivers reach the sea.

Scenic destinations (all `--exagg 1`):

| place | args |
|---|---|
| temperate river valley (wade in!) | `--lat 4.990 --lon -29.403 --alt 0.3` |
| great-river delta, jungle | `--lat -0.49 --lon -30.55 --alt 6` |
| great-river mouth, steppe | `--lat 7.042 --lon 33.477 --alt 8` |
| inland sea (Caspian-scale) | `--lat 20.633 --lon 127.615 --alt 80` |
| lake archipelago | `--lat 16.601 --lon -12.222 --alt 12` |
| desert river | `--lat 0.87 --lon -89.80 --alt 1.5` |
| temperate groves + ponds | `--lat 3.96 --lon -32.56 --alt 0.05` |
| taiga + cave pits | `--lat 51.18 --lon 85.83 --alt 0.05` |
| savanna acacias | `--lat 1.51 --lon 40.49 --alt 0.05` |
| highest peak (8.58 km) | `--lat 69 --lon 122 --alt 1` |

## Phase 3.5 (bug round, 2026-07-06)

* the sea is now classified by the map's real ocean mask (koppen 255,
  blurred), not `elevation <= 0` — no more navy plates on coastal land, and
  the planet's genuine below-sea-level *dry* basins render as dry basins
* chunk selection samples at the local gnomonic chunk size — chunks near
  cube-face edges/corners (half/third size) no longer vanish
* far-mesh shading agrees with the block materials: snow starts at the same
  -9 C treeline (dithered on blocks), rock shows on steep ground instead of
  washing whole rough biomes gray, frozen water paints as ice
* rivers can't tower: where fine relief dives below the regional water
  level the channel fades to a dry wash instead of a wall of water
* walk mode is real physics (gravity, landing, head bumps, side collision
  with 1-block step-up, slow-sink wading with Space to swim); fly descends
  to 2.5 m and into cave pits
* physics is tuned for `--exagg 1` (at 10x, blocks are 10 m tall and jumps
  can't clear them)

## Phase 6 (sky, feel & polish — done)

* **Atmosphere**: a real sky — day gradient with bright horizon band, sun
  disc and glow, warm low-sun sky (`--sun-lat 0 --sun-lon <far>` for
  sunsets), and it thins to black space as you climb to orbit. Terrain
  fog now melts into the same horizon color.
* **Pointer lock**: click captures the mouse for raw free-look, Esc
  releases (Esc again quits).
* **Block ambient occlusion**: corners darken under neighboring blocks —
  the classic soft shadow that makes voxels read as 3D.
* **Cave darkness**: faces under rock overhead dim with depth (pit floors
  open to the sky stay lit).
* **Tree trunks are solid**: bump into them, land on them (shrubs and
  leaves stay passable). Physics and rendering share one tree decision.
* **Edits persist** per planet seed; **salt lakes** tint mineral-pale
  (a frozen salt lake renders as ice — the ice ramp wins).

## Phase 6.5 (bug round, 2026-07-07)

* **Sky no longer blacks out near the ground at sunset**: the sky pass
  unprojected the far plane through the f32 inverse view-projection; below
  ~8 m altitude the terms cancelled and every ray flipped. The renderer is
  now reversed-Z with an infinite far plane and the sky unprojects the
  near plane — robust at any altitude.
* **You carry a torch**: cave darkness is applied in the shader instead of
  baked into the mesh, and a warm light pool around you (~10 m) pushes it
  back. It only acts where it's dark — daylight terrain is untouched.
* **No more seeing inside tree trunks**: reversed-Z lets the near plane
  shrink to ~14 cm at eye height, and the walker got a 0.35-block body
  radius (8 probe columns ring every step) so the eye keeps clear of walls.
* **Towers stay what you built them from**: materials, steepness, and cave
  bands anchor to the *natural* (pre-edit) ground — a tower on grass is a
  dirt tower with a grass cap, not a stone cliff. Editing a tree's column
  chops the tree (it no longer rides your blocks or pops back).
* **Placing is face-aware**: aiming at the side of something builds on the
  column in front of it; aiming down at a top face grows that column. (In
  the column-edit model a placed block always lands on its column's top —
  you can't float blocks mid-air yet.)
* Headless `--capture` now loads your saved edits, so screenshots show the
  same world you play in.

## Phase 7a (geomorphing & the patch boundary, 2026-07-07)

* **Geomorphing**: every mesh vertex carries the height and river-paint
  wetness its PARENT level would give it; the vertex shader slides between
  the two over a distance band derived from the tile-selection threshold,
  so every LOD swap exchanges identical geometry and identical paint —
  seamless by construction (including the level-dependent painted river
  width, which used to halve in one frame at each split).
* Honest measurement (`examples/popdiff`, a frame-diff pop meter with tile
  -swap instrumentation): raw geometry pops were ALREADY sub-visible at
  cruise altitude — the band-limited octave scheme means a swap only adds
  detail that is ~1 px at its swap distance. The morph turns that
  statistical near-seamlessness into a guarantee, and retires the river
  pop. Costs: +1 sample pass per tile only where octave budgets differ,
  +8 bytes/vertex.
* **Patch boundary**: the voxel patch used to end on a floating one-block
  cliff (blocks ride ~1.6 m proud of the mesh). Rim blocks now sink flush
  with the mesh across the outer ~15% of the patch — the blocks end in a
  feathered shoreline.
* Dev switches: `TRI_NO_MORPH=1` (raw levels), `TRI_FORCE_MORPH=1` (render
  every tile as its parent's geometry — a morph sign/scale bug shows as
  spikes immediately).

## Phase 7b (night sky & torches, 2026-07-07)

* **Stars**: a hashed directional star field that owns the dark — night
  ground and open space — and dims away under any sky light.
* **Limb glow**: from orbit the planet wears a thin sunlit blue rim of
  atmosphere (ray closest-approach against the surface radius).
* **Placeable torches (R)**: toggle a torch on the block you aim at. The
  torch is real geometry (crossed quads, emissive flame) and a real point
  light — warm pools at night, pushed-back darkness in caves. The nearest
  16 torches light each frame; they persist per seed and ride column
  edits.

## Phase 7c (day & night, 2026-07-07)

* **Day/night cycle**: the sun stands still and the planet turns — local
  time depends on longitude, the terminator crosses the planet from
  orbit, sunsets happen to you instead of being launch options. Default
  20-minute day (`--day-len`, 0 = legacy always-noon).
* **Torch flames flicker** (a time uniform breathes both the emissive
  quads and the point-light intensities, each on its own phase).

## Phase 7d (river LOD paint, 2026-07-07)

Distant rivers now paint at **coverage-correct opacity**: the corridor is
still widened to one vertex spacing for continuity, but tinted only as
strongly as the real channel fills it. Sub-vertex tributaries fade into
faint threads instead of full-strength bands — which is what made every
confluence bloom into a blob — while great rivers keep their weight and
fade naturally with distance. Ponds' feathered wetness fades the same
way. (Diagnosed while verifying: the *angular blue polygons* at coarse
LOD are lake-cell footprints — geometry, not paint — a separate item
below.)

## Phase 7 (remaining)

Swimming polish, rapids/waterfalls where river levels step, lake
footprints alias to angular vertex polygons at coarse LOD (needs a soft
distance-to-shore signal from the lake index), a moon and night ambient
lift, river ribbons as real geometry at coarse LOD (the endgame for
distant rivers).
