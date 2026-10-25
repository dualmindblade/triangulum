# Iteration log — terrain roughness session (2026-07-05)

Autonomous tuning session while you two are out running. Goals, from your notes:

1. SST-ITCZ coupling (banked idea, greenlit — decoupled from the rest, done first).
2. Diagnose + fix the directional bias in the hydrology initial state
   (`interchange/hydrology-initial-state.png` — parallel diagonal grooves).
3. Terrain roughness: post-erosion roughness should match or exceed the seed
   terrain; direction = the Himalaya photo (`interchange/mountainvalleysonearth.png`).
4. If all good: resolution bump to level 8.

Every run recorded (`simviz/player.html` in each output dir). This file logs each
step, what changed, what I measured, and what I saw.

---

## Step 1 — SST-ITCZ coupling  [DONE]

**Change:** ocean convective rain now multiplied by
`clip(1 + sst_itcz_coupling * SST_anomaly/3.5, 0.3, 2.2)` where the anomaly is
SST minus its zonal mean (i.e., "warm for its latitude"), interpolated per
step like the other monthly fields. New config: `sst_itcz_coupling = 0.6`.

**Result (L6, output/iter_sst):** the equatorial rain band now varies along
its length — flares near warm pools, thins over cooler water — instead of
being a uniform stripe. Land precip calibration unaffected (826 mm, smoke
test). Screenshot: scratchpad/sst_coupling.png.

---

## Step 2 — seed-terrain striations (your hydrology-initial-state screenshot)  [DONE]

Chased through three hypotheses, keeping each change that helped:

1. **Two-tier domain warp** — the strong continental warp (amp 0.35 ≈ 2200 km)
   shears fine noise octaves into aligned grooves. Mountain/hill/abyssal/
   hardness/chain noise now uses a gentle warp (amp 0.06). Improved isotropy,
   but the diagonal brushing persisted → not the main cause.
2. Split base_int / era-mod / sag noise the same way → no visible change.
3. **The real culprit: distance-transform facets.** Graph-geodesic distances
   propagate as straight cones along local lattice directions; every kernel
   built on them (orogen falloffs, crust-edge taper, seafloor age, ancient
   ranges) inherits fan-shaped striations — visible in the ocean too, and
   exactly what the rivers were following. Fix: `Grid.rough_metric()` —
   seeded symmetric per-edge length noise (±27%) applied to all tectonic
   distance transforms, so fronts propagate irregularly like real geology.

**Result:** the parallel grooves are gone from the seed terrain (zoom check:
scratchpad/warp_check.png); ocean fan artifacts dissolved as well. So the
river bias was indeed the seed's valleys, as suspected — the meander/capture
routing keeps working, but now on honest terrain.

---

## Step 3 — roughness dynamics (goal: final roughness >= seed)  [DONE at L6]

New metric printed every run: mean local relief (std of elevation over each
cell's neighborhood) on land, seed vs final. Iterations, all at L6 with the
stage cache (`--from hydrology`, ~25 s each):

| iteration | change | roughness seed -> final |
|---|---|---|
| 1 | fine ridged noise in orogens + uplift (freq x6), fine erodibility contrast, slope-gated diffusion, deeper network incision (acc_ref 1000->350) | 241 -> 200 m (-17%) |
| 2 | deposit_fraction 0.55->0.35, diffusion_k 0.015->0.010, grit 0.035->0.05 | -> 221 m (-8%) |
| 3 | exposed-bedrock texture (signed, post-erosion, 0.22 km) | -> 237 m (-2%) |
| 4 | texture 0.30 km | -> 241 m (0%) — but signed texture minted thousands of tiny lakes (speckle) |
| 5 | inject texture 25 steps before end so rivers breach it + filter shallow 1-2 cell ponds | speckle reduced, not enough (300 m hollows survive 25 steps) |
| 6 | **ridge-only texture** (positive crests, no hollows) | **241 -> 241 m (+0%), speckle gone** |

Slope-gated diffusion is the keeper dynamic: steep faces shed mass (talus),
gentle terrain keeps its grain instead of blurring — this is what stopped the
"everything gets smoother over time" trend at its source. The dissected-look
texture rides on thin-soil uplands only; sediment plains stay flat, as they
should.

---

## Step 4 — L7 validation  [DONE]

`output/seed42_r7_fib` (recorded + videos, 17 min). Roughness now **grows**
through erosion: seed 141 m -> final **157 m (+11%)** — the landscape creates
relief as it evolves instead of blurring. Dense branching valley dissection
across the continental highlands; rivers wind and merge; lakes cluster in
lowland basins and mountain valleys rather than speckling everywhere.

## Step 5 — L8 showcase (resolution bump)  [DONE]

`output/seed42_r8`: 655,362 cells, ~31 km spacing, 71 minutes total
(tectonics 3.4 min, each climate pass 31 min, hydrology 4.3 min, render 26 s).
Roughness: seed 92 m -> final **126 m (+37%)** — the relief-creation trend
strengthens with resolution, as it should: finer cells resolve more of the
drainage that does the carving.

Zoomed comparisons against the Himalaya goal photo saved to
`interchange/l8-range-zoom.png` and `l8-range-zoom2.png`: ice plateau with
glacial lakes, dissected flanks, radiating dendritic drainage.

### Roughness across resolutions (seed -> final)

| level | cells | seed | final | change |
|---|---|---|---|---|
| 6 | 41k | 241 m | 241 m | +0% |
| 7 | 164k | 141 m | 157 m | +11% |
| 8 | 655k | 92 m | 126 m | **+37%** |

(Seed roughness falls with resolution because each cell spans less terrain;
what matters is the sign of the change — erosion now *adds* relief.)

---

**Session totals:** ~15 pipeline runs (mostly L6 + cache), 3 recorded
showcases (L7 ico-era baseline earlier, L7 fib validation, L8 final). All
changes smoke-tested; docs updated (DESIGN.md, README, config comments).

---

# Game phase — Phase 0 viewer (same day)

Decisions: Rust + wgpu, hybrid far-field, gnomonic cube-sphere prisms
(arbitrary grid dims OK — 10M = 32 x 312,500, zero partial chunks; upper
hierarchy mixed-radix or cropped power-of-two cover). Cubyz (C:\code\Cubyz,
Zig, GPLv3) verified to generate chunks directly at each LOD — the
hypothesized architecture is real and shipping.

Built `viewer/` (Rust): cube-sphere quadtree LOD renderer over the baked L8
planet rasters (`scripts/bake_faces.py`). Proven: per-tile independent
generation from (face, level, ix, iy), f64 camera-relative rendering (no
jitter at 8,660 km), screen-error LOD from 20,000 km orbit down to the raster
floor (~level 5), headless capture mode. Screenshots:
`interchange/neisor-orbit.png`, `neisor-mid.png`, `neisor-low.png`.

Notes for next session: wgpu 30 API notes are embedded in the code (Option-
wrapped pipeline fields, `queue.present`, `PollType::wait_indefinitely`,
display-handle instances); glam camera helpers deprecated in favor of
`glam::dcamera` (warnings, non-blocking). Phase 1 = Rust port of
planetgen/noise.py + octave bands below the raster floor + threaded tile
builds + first voxel chunks.

## Phase 1 (same day) — seams fixed, noise parity, detail octaves

User found a hairline crack (interchange/neisor-seam.JPG). Three fixes:
1. cull_mode None — skirt quads had mixed winding; culled skirts reopened
   the cracks they exist to hide (why it was angle-dependent/hard to find).
2. Edge-inclusive face rasters (linspace -1..1) + matching sampler — kills
   the half-texel color/elevation seam along all 12 cube edges.
3. Ghost-ring normals (one extra sample ring per tile, central differences
   everywhere) — kills lighting seams at every tile border.

Phase 1 core landed:
- `viewer/src/noise.rs`: exact port of planetgen/noise.py; gradient table
  exported from Python; `cargo test` proves 240 golden values match to 1e-9
  including int64-overflow hash paths (wrapping_mul == numpy wraparound).
  SAME SEED = SAME PLANET across Python and Rust, verified.
- Band-limited detail octaves below the raster floor (`fbm_band`/
  `ridged_band`): each quadtree level adds exactly the octaves its vertex
  spacing carries; MAX_LEVEL 5 -> 12 (~100 m vertices). Amplitude enveloped
  by raster elevation (mountains rough, plains calm), land only.
- rayon-parallel tile builds.

Verified by capture: no face-boundary seam at lon 45 (2,000 km), procedural
relief with working envelope at 150 km, continuous cross-tile shading at
25 km. Screenshots: neisor-seamcheck/detail/detail2.png in interchange/.

Known Phase-2 items: koppen color texels are blocky up close (needs biome
splatting/dither + finer climate raster), detail pops when tiles split
(geomorphing), sun is fixed (add --sun or time-of-day), no atmosphere
scattering yet, and the big one: first true voxel chunks near the camera.

## Phase 2 first slice (2026-07-06) — THE BLOCKS ARE REAL

`viewer/src/voxel.rs`: 10,000,000-column lattice per face (1 m columns, the
game-spec dimensions; 10M = 32 x 312,500 so chunks tile exactly). Chunks of
32x32 columns mesh as quantized tops + exposed sides between 1 m radial
shells — the diamond prisms. Full-depth octave stack (12 octaves, ~3 m
floor) for block heights; rayon-parallel builds; separate GPU cache; drawn
with a small radial lift over the heightfield (hybrid boundary v1).

Debugging story worth remembering: first block captures were identical
garbage from two *different* camera bugs — (1) look-at up-vector nearly
parallel to the look direction at mid latitudes (degenerate view basis), and
(2) the real one: camera altitude was measured above sea level, so at 10x
exaggeration we were 10 km INSIDE a mountain looking up at its underside.
Altitude is now relative to the local surface (`terrain::ground_height_km`),
the view tilts toward the horizon as you descend, and the sun follows the
camera by default (--sun-lat/--sun-lon for fixed).

Captures: interchange/neisor-blocks.png (hybrid handoff visible),
neisor-blocks-close.png (60 m up, real scale — quantized terraces with
per-block shading). Phase 3: mountains tour, boundary blending, block AO,
biome palettes, face-crossing rings, first block edit.

## Free look + player motion (2026-07-06)

Camera reworked to yaw/pitch free look in the local tangent frame (drag =
look, --yaw/--pitch for captures, auto-pitch by altitude when unspecified).
WASD flight relative to heading with altitude-scaled speed (Shift sprint).
**Walk mode (G)**: eye 1.8 m above the rendered block tops —
voxel::surface_height_km mirrors the mesher's quantized shell + lift so feet
match the visible blocks — 4.3 m/s ground-following movement, ballistic
jump on Space (5.2 m/s, 9.81 gravity), F returns to flight. Window title is
a mini HUD (mode, lat/lon/alt, twice a second).

Rust note for the log: the borrow checker rejected holding the `gfx` borrow
across the whole event handler once update() needed &mut self — the fix
(borrow per match arm) is exactly the discipline that prevents the
use-after-free this pattern invites in C++.

Verified: interchange/neisor-ground-view.png (pitch -4, yaw 220 at eye
height — block terraces to a planetary horizon). Golden noise tests still
green. Known gaps: no pointer-lock mouse yet (drag only), no side collision
(walking up a cliff face is currently legal alpinism), chunk ring still
clamps at face edges.

## User bug batch + first edits (2026-07-06)

1. **Spiral-to-pole (loxodrome)**: holding W kept a constant compass
   bearing = rhumb line into the pole. Camera::translate now
   parallel-transports the heading along the motion great circle and
   recomputes yaw in the new frame — flight follows great circles.
2. **Mesh/voxel interpenetration** (user screenshots): tile heights used
   ~6 octaves at L12, voxels 12 — fields disagreed by ±15 m in mountains,
   dwarfing the lift. Fix: "deep" tiles — leaves at level >= 9 within the
   voxel footprint sample the full VOXEL_OCTAVES stack (TileKey.deep), so
   mesh and blocks agree to <= 1 block; lift shrunk to 1.6 blocks. Verified
   at the planet's highest range (8.58 km peak, lat 69 lon 122 — found by
   probing the baked rasters; earlier "mountain" coords were open ocean and
   the scary pyramid was just a smooth island cone).
3. **View flip at nadir**: up-vector switched discontinuously near
   straight-down. Now: pitch clamped to +/-86 deg, always-radial up.
4. **Descent cinematic restored**: scrolling eases pitch toward an
   altitude-appropriate target (planet-gazing in orbit -> horizon low),
   while drag remains fully free.
5. **First block edits**: Q breaks / E places the top block of the
   raycast-targeted column (column-delta model, voxel::Edits sparse map);
   touched chunks + border neighbors are invalidated and remeshed; walking
   height respects edits. Interactive-only — needs the family's hands-on
   test.

Note: the interchange/ screenshot folder moved into viewer/interchange/
(user relocation) — captures now write there.

## Second round: one pixel, one system (2026-07-06)

User feedback: (a) scroll auto-tilt asymmetric (ascending lagged), (b) once
descended to horizon view the camera should be fully free like on the
ground, (c) block faces inconsistent with the mesh (screenshot
second-round-mesh-issues.png) and doubt that voxels + mesh belong in one
scene.

Diagnosis from the screenshot:
- the smooth dark-green patches between block tops were the *heightfield
  poking through* wherever a coarser (non-deep) tile still covered part of
  the voxel patch - its height error beats the 1.6-block lift;
- the black stair-step cracks were block *side faces*: horizontal normal,
  sun near-overhead, lambert ~0 with only 0.10 ambient = void-black;
- the tone jump at the patch edge: blocks painted koppen x0.8 vs the
  mesh's x0.55.

Fixes (decision: keep the hybrid, but stop *layering* it - CUT the mesh
away under the voxel patch so every pixel belongs to exactly one system):
1. Renderer passes a "hole" disc (camera ground point + conservative
   radius from voxel::safe_hole_radius_km - two chunks inside the built
   ring, shrinking near face edges) and the fragment shader discards
   heightfield pixels inside it (voxel chunks flagged exempt via
   offset.w). Poke-through is now impossible by construction, whatever
   the octave counts do. The cut edge hides under the block ring.
2. Block side normals tilt 0.85 toward up (Minecraft-style: sides shade
   to ~half the top brightness, never black); side tint 0.62 -> 0.72.
3. Block palette matched to the mesh (koppen x0.58) so the patch edge no
   longer reads as a brightness cliff.
4. Scroll auto-tilt is now a pure function of altitude (identical up and
   down = symmetry fixed) and fades to zero below 100 km: near the ground
   the pitch is entirely the player's, exactly the requested "freed up at
   the horizon" behavior. Above ~800 km it pulls at full strength (0.35).

Verified: interchange/fix-plains-low.png + fix-plains-down.png (no
poke-through, no black cracks, clean cut fully covered by blocks straight
down), fix-peak-walk.png (snow blocks in soft greys at the 8.58 km range).
Golden noise tests green. Remaining cosmetic: a visible texture band where
blocks hand off to mesh at the patch edge (boundary blending is on the
Phase-3 list), and the block field still pops in when crossing 2.5 km.

## The procedural generation night (2026-07-06, autonomous session)

User: fix the face-boundary chunk snap now; then "beef up the procedural
generation" with authority to decide unilaterally - varied landscapes,
biome-real block types/colors, roughness-driven mountains, rivers, beaches,
ponds, trees (full blocks only), caves and overhangs; quash rendering bugs
surfaced along the way. ~24h of session time available, tokens plentiful.

### Face-boundary chunk rings (first ask)
select_chunks now samples a tangent-plane disc and asks face_from_dir which
chunk owns each sample - the ring spills across cube-face edges for free.
Ghost columns past an edge resolve to the neighbor face via the extended
lattice direction (cube-face lattices coincide along shared edges, so
cross-face side quads meet the neighbor's tops exactly). safe_hole_radius_km
no longer shrinks at edges. Verified straight-down and at ground level on
the lon-45 edge: full disc, no snap, no holes (edge-before/after*.png).

### Data: 4 new baked fields
bake_faces.py now emits rough_km (mean |elevation delta| to graph
neighbors), precip_mm_yr, temp_c (annual means), flow_log10 (river flow
accumulation, 3NN max so lines stay fat) - face_*.bin is n*21 bytes now
(REBAKE REQUIRED: python scripts/bake_faces.py output/seed42_r8 1024).
planet.rs grew bilinear accessors for all four.

### Generation v2 (terrain::sample returns a Sample struct)
- Roughness-driven relief: detail envelope 0.06 + rough*0.85 + e_raw*0.10
  (clamped 0.05..1.7), ridged fraction also scales with rough - mapped-rough
  country is jagged, plains stay calm. Roughness is damped near sea level
  (continental-slope spike would put cliffs on every beach).
- Rivers: ribbon channels along the zero-set of a wandering 2-oct noise
  field (freq 1400), gated by flow_log10. Water level rides the 4-octave
  terrain (locally calm); the channel carves a GORGE through fine relief so
  rivers stay continuous across knolls. Extent = f(channel field only), so
  every LOD level agrees where water is.
- Ponds: noise depressions in wet calm lowlands, filled to just below the
  original ground line. Sea: geometry at 0 km, shallow block floor.
- Water rendering: sea is real geometry; inland water is PAINT. Vertex
  carries (water color, wetness); deep tiles step() the wetness for crisp
  per-pixel edges, far tiles blend a soft tint (a sub-vertex ribbon lifted
  per-vertex tents into floating shards - color can't). Blocks render true
  3D water with matching depth-tinted color.
- Caves: tube network = intersection of two gradient-noise level-sets,
  sampled with a radial offset (dir * (freq + z/scale)) for true vertical
  variation; 26 blocks deep under dry land, bit-per-block per column.
  Mouths breach the surface as pits/overhangs. Column model is now
  ground + cave_bits + water, meshed generically (tops, bottoms, exposed
  side runs split by material).
- Materials: Grass (biome-tinted), Dirt, Sand, Gravel, Stone, Rock, Snow,
  Ice, Water, Log, 4 Leaves kinds, Shrub. Surface by koppen/temp/steepness
  (cliffs bare stone/rock, beaches sand, deserts sand over stone, snow
  under -9C or EF), substrate strata under the surface. ground_tint() maps
  all 30 Koppen classes to naturalistic colors, shared by mesh shading and
  blocks - the koppen debug palette is retired from the world (the planet
  is green/tan/white from orbit now).
- Trees: hash-placed per column; kind+density by biome (jungle canopy 0.011
  /col ~ closed forest, taiga conifers, broadleaf temperate, acacia savanna,
  shrubs steppe/tundra); treeline by temperature; no trees on slopes,
  carved gullies, cave mouths, beaches, or water. Shapes are full-block:
  conifer diamond-stack, broadleaf/jungle blob (hash-ragged), acacia flat
  top, 1-2 block shrubs. Chunk meshing scans anchors in a 4-column margin
  so cross-chunk canopies are identical on both sides; occupancy-aware face
  emission (bottoms included - canopy undersides render).

### Bugs found by looking (the point of the exercise)
1. Tree "orchard rows" + constant heights + weird flat leaf-blob clusters:
   hash_u64 was a LINEAR function of (ci,cj) - no avalanche; thresholding
   it selects lattice stripes. Fixed with a splitmix64 finalizer. This
   single fix turned plantations into natural clumped forest.
2. Mesh water at coarse LOD: per-vertex geometry lift tents isolated wet
   vertices into floating navy shards -> the paint approach above.
3. Water level from local bumpy terrain made rivers stepped crater chains
   -> level from the 4-octave base + gorge carve.
4. Roughness spike at coasts (see above).
5. Block water color used a blocks-scale depth ramp vs mesh's km ramp -
   patch boundary jumped shade. Unified.
6. Trees in shallow river gullies poked leaf scraps out at rim level -
   anchors reject carved ground anywhere under the canopy footprint.

### Verification
Biome tour (all in viewer/interchange/): final-temperate-grove, final-taiga
(conifers + cave pits), final-savanna (acacias + lake), final-jungle-river
(delta), gen11-icecap, gen2-desert (dunes), gen10-orbit (natural-color
planet), sweep-pondhunt1 (pond), gen11-river-mid (desert river canyon,
mesh->block continuity). Golden noise tests still pass. examples/probe.rs
added as a generation-context debug tool.

### Known gaps (punch list for next sessions)
- Walking stands ON water surfaces (no swim); side collision still absent.
- Far-tile inland water is angular at vertex resolution; rivers pop from
  soft tint to crisp near the patch. Distant forests absent from the mesh
  (could darken tint by biome).
- Cave interiors unlit-but-visible (ambient 0.10); no torches yet.
- Trees pop with chunks; no geomorphing; patch edge still a texture band.
- Editing a column under a tree moves the tree (column-delta model).
- Block AO, biome-specific stone/ore palettes, birch/palm variants: later.

### Late additions (same session)
- Wading: water is no longer walkable - you sink to the floor of ponds and
  rivers and walk out; when the eye dips below the surface the whole view
  tints underwater blue (Globals flag -> shader mix). Verified standing on
  a river floor: water surface reads as a ceiling overhead.
- Distant forests: mesh ground tint darkens by biome tree density, so the
  tree-covered voxel patch no longer pops out of a flat bright lawn - the
  patch/mesh boundary is now hard to spot tonally at altitude.
- examples/probe.rs stays in the tree as a generation-context inspector
  (koppen/temp/flora decisions around any lat/lon).

## The morning-after bug round (2026-07-06, day session)

User field report (viewer/interchange/notes-on-improved-terrain-gen.txt +
8 coordinate-stamped screenshots): rivers standing dozens of blocks over
the terrain, chunks vanishing near "certain areas", blue tiles on coastal
land, plus three feature asks (in-game screenshots, teleport, real gravity).
Every bug reproduced headlessly from the stamped coordinates before fixing.

### Root causes (three of them were "mesh and blocks disagree")
1. **Sea was `elevation <= 0`.** The map has genuine dry below-sea-level
   basins (13k texels, down to -4.3 km!), and bilinear elevation dips a few
   meters under zero on land all along the coasts. Every such vertex got
   navy paint + geometry lift -> plates on beaches, whole "seas" over
   deserts, while the blocks stayed dry sand. Fix: Planet now carries a
   blurred is-ocean mask (koppen 255, radius-2 box blur at load);
   sea := e_raw <= 0 AND ocean_frac > 0.35. Interior basins keep their
   true depth (h floor only applies within a texel of real ocean).
2. **Chunk selection undersampled near cube-face edges.** The tangent-disc
   walk sampled every 22 m, sized for 55 m face-center chunks - but the
   gnomonic cell shrinks by 1/(1+u^2+v^2): 28 m chunks at edge middles,
   18 m at corners. Missed chunks = see-through holes to the far-side
   ocean (user: "planet visible thru broken chunks"; their lat 39.250 /
   lon 35.213 sits EXACTLY on the +Z/+X face edge - atan(cos 35.2) =
   39.25). Fix: step = 0.45 x local chunk size.
3. **Mesh snow started at +1 C, blocks at -9 C.** Between those, the far
   mesh rendered near-white against olive blocks - beyond the patch edge
   the world "disappeared" into what read as sky. Same family: mesh rock
   washed whole rough biomes gray (blocks only rock on steep columns), and
   frozen water painted navy where blocks froze to ice. Fixes: one snow
   threshold (-9 C, mesh ramps -7.5..-10.5, blocks dither the same band),
   rock now needs actual slope (from the vertex normal), water paint goes
   ice-color below -4 C.
4. **Perched rivers.** Water level rides the 4-octave base; in rough
   country (jungle site: rough 0.75-0.85) the full-octave terrain dives
   tens of meters below that base, and the channel stood as a stepped
   water wall (user's "towering river" at lat 4.971 lon 9.479). Fix:
   perch = smoothstep(1..5 m) of (water level - natural terrain); perched
   channels fade to a dry wash. Max exposed water wall is now ~3 m.

### Features
- **Real walking physics**: vertical velocity + gravity (terminal -80 m/s),
  landing on cave-aware support (voxel::support_below_km), head bumps on
  cave roofs (ceiling_above_km), side collision by body-span test with
  1.05-block step-up, water = slow sink + hold-Space swim. G in flight =
  skydive. F fly descends to 2.5 m and sinks into cave pits (absolute
  height preserved near ground so pits don't yank the camera; roofs solid).
- **T teleport**: title-bar prompt, "lat lon [alt]", Enter/Esc.
- **P screenshot**: PNG into interchange/ with lat/lon/alt/yaw/pitch baked
  into the filename - user screenshots now carry their own repro command.

### The hydrology question (user asked: are rivers using the flow map?)
Honest answer: courses are noise (fbm_band zero-set); the baked flow_log10
only gates where/how wide. Worse, the bake dilates flow with a 3NN max, so
the "near a river" gate is a 40 km plateau (probed: flow ~2.17 constant
across the whole box at the tundra site) - ribbons can spawn anywhere in
it. planet_data.npz already has the real drainage graph: receiver (int32
downstream pointer per cell), river flags, flow_accum_m3s, lake_id/salt.
Options written up for discussion in the wrap-up message; recommendation:
ship river polyline segments (cell->receiver, ~600 KB) + a viewer-side
bucket index, query exact distance-to-course in sample(), meander with
noise displacement. Lakes get real extents/levels the same way.

### Anomalies triaged, not bugs
- "squishedvoxels": gnomonic anisotropy - columns are ~1.7 x 1.0 m at face
  centers (squat), ~0.9 x 1.0 m near edges (thin). The 10M-columns-per-face
  spec is the design; changing it is a family decision.
- Black sky in that shot: space. No atmosphere scattering yet, so above
  the horizon is black even at noon. Moved up the Phase 4 list because
  users keep reading it as a rendering bug.

### Verification
repro-holes-y0/y180 (before), fix-holes-y0/y180, fix-tower/tower-y0..270,
fix-basin, fix-coast, fix-plates1/2, fix-canyon/-s (all in
viewer/interchange/). examples/mapprobe.rs (raster + sample transects) and
examples/physim.rs (collision-query invariants, 400-column transect, 14
cave breaches, all asserts green) added as debug tools. Golden noise tests
pass. 10 s live-window smoke test at the taiga cave site: no panics.

### Addendum: the blue coastal tiles, act two (same day)
User: still seeing blue tiles on desert coasts. Three classification fixes
in a row failed to kill them (landward depth-bias, then a flood-fill
ocean-connectivity mask — the sub-zero region is a connected TONGUE, not a
closed pocket, so connectivity cannot separate it). An ASCII map of the
classification (examples/seamap.rs) finally showed the truth: the plates
were never water. They are koppen==255 (ocean) texels where interpolated
elevation OVERSHOOTS above zero — legitimate land — and ground_tint's
fallback arm painted them [0.02,0.09,0.18] "ocean floor" navy, while the
blocks' fallback picked sand. The user's original description ("water
imprecision colored sand on the ground") was exactly right, and their
second guess (a koppen color) was right too. One-line fix: the fallback
tint is now coastal-strand sand, matching the blocks.
Kept from the failed attempts (all genuine improvements): sea is
classified by the map's sharp cell-resolution coastline (bilinear
koppen-ocean mask >= 0.5) with a deep-water-near-coast override for
mislabeled strait/fjord texels; interior basins stay dry; the sea carries
no wet-paint (it bled navy onto beach triangles); shallow sea (< ~20 m)
shoals to teal on both mesh and blocks.
Verified: plates-steppe-fixed5 / plates-desert-fixed5 (strands render as
sand), plates-delta-check (rivers unchanged), golden tests green.
Lesson recorded: when a fix does not change the artifact AT ALL, stop
refining the hypothesis and go look at the classification itself
(seamap.rs exists now for exactly that).

## Rivers from the map (2026-07-06, continued day session)

User approved the drainage-graph plan ("let's do rivers"). Noise no longer
invents courses; it only decorates them.

### Pipeline
- scripts/bake_rivers.py: exports cell->receiver segments (flow > 120 m3/s,
  planetgen's own river flag starts ~350 — the extension lets headwaters
  taper in), water levels relaxed monotonic downstream (59 passes; 277 raw
  links ran uphill through filled depressions, worst 653 m), node smoothing
  (2 rounds of pull-toward-neighbor-midpoints kills the 30 km polyline
  kinks) with levels RE-ANCHORED to the terrain under the smoothed
  positions (forgetting this dried whole reaches: the perch guard saw
  water above ground), plus lake cells with spill levels and RIM cells.
  -> viewer/assets/rivers.bin (1.2 MB).
- viewer/src/rivers.rs: per-face uv bucket grid (segments inserted within
  their influence radius on every face they touch, so queries read exactly
  one bucket; empty bucket = free "no river here"). Nearest-segment query
  returns distance / lerped level / flow. Lakes: a point is in the lake iff
  its nearest map cell is a lake cell, not a rim cell — Voronoi footprint.
  Per-cell discs first rendered lakes as overlapping circles.
- terrain.rs sample(): meander = bounded noise displacement of the query
  point (0.18 km, always inside the damped floodplain); valley damping =
  fine-relief envelope fades to 12% near the channel (exact distance makes
  this surgical — the old fuzzy flow-plateau gate would have flattened half
  the map); channel = parabolic bed below the graph water level, banks
  widen with cut depth (ridge crossings become canyons, not km-wall slots);
  width/depth from hydraulic geometry (w ~ 3 sqrt(Q) m, d ~ 0.27 Q^0.39 m);
  perch guard kept as backstop. Lakes fill to spill level and render like
  the sea (geometry at the surface + water ground color — a painted 500 km
  lake would read as a blue bowl). LOD-aware paint: far tiles widen the
  painted thread to one vertex spacing (a thread narrower than the spacing
  only caught scattered vertices and shattered into shards).

### What it looks like
Rivers meander through flat-bottomed valleys with riparian trees, grow from
creeks to a 700-m-wide Amazon-class mouth, and reach the sea. The
55,000 m3/s great river happens to run through the old jungle-delta scenic
spot. Lake 413 is a Caspian-scale inland sea with bays and islands
(verified against the planetgen physical map). Walking into a river wades/
swims it (existing water physics picked this up for free).

### Retired
The fbm ribbon field, the flow_log10 raster gate (probed: a 40 km constant
plateau around every river), and the "rivers wander anywhere near water"
look. planet.flow() stays for probes.

### Known aliasing (Phase 6 list)
Distant painted threads are one-vertex zigzags; confluences blob at coarse
LOD; lake/sea shorelines staircase at far vertex spacing; no rapids/
waterfall rendering where the level steps; salt lakes look like fresh ones.

### Verification
riv-* captures in viewer/interchange (grand mouth, delta v1..v3, mid-river
blocks at the smoothed course, lakes 413/526 before/after Voronoi, old
canyon site). Golden noise tests green. Scenic table in README updated
with data-derived coordinates (bake_rivers.py prints the grandest rivers
and largest lakes with lat/lon on every run).

## Phase 6: the sky and the feel (2026-07-06, evening session)

One pass over the "make it feel like a game" list. Geomorphing deliberately
deferred (riskiest item, deserves its own session).

### Atmosphere
A sky pass: fullscreen triangle drawn at the far plane in the same render
pass (LessEqual depth, z=w — it only wins where no terrain drew). Globals
gained the inverse view-projection (camera-relative space means an
unprojected far-plane point IS the view ray), camera radial up, and camera
height. Sky = day gradient (bright horizon band -> zenith blue), sun disc
+ glow, warm sky around a low sun, fade to space-dark below the horizon
line, and exp(-alt/45km) so the atmosphere thins to black space by orbit.
Terrain haze replaced with aerial perspective toward the same horizon
color. Verified: ground/9km/orbit captures (p6-sky-*, p6-orbit — planet
floats in black space), and p6-sunset (sun disc on the horizon with warm
glow) via --sun-lat/--sun-lon.

### Feel
- Pointer lock: click captures (Locked, falls back to Confined on
  platforms without it), raw DeviceEvent::MouseMotion drives look, Esc
  backs out one layer (capture, then quit). Drag-look stays as fallback.
- Block AO: classic per-corner occlusion on top faces from the three
  blocks touching each corner one level up (0/1/2-side + corner rule).
  quad() takes per-corner colors now.
- Cave darkness: every face dims by (top_solid - z) — depth below the
  walkable top. Pit floors are their column's top_solid, so floors open
  to the sky stay lit; interiors under rock go to 25% by ~4 blocks.
- Tree trunks solid: shared voxel::tree_here() applies the SAME
  slope/gully/cave-mouth rejections as chunk meshing (col_ctx_ext is now
  the shared extended-lattice column lookup), and support/ceiling queries
  treat trunk blocks as solid. Shrubs and canopy stay passable. physim
  extended: landing on trunk tops asserted across two transects.
- Edits persist: assets/edits_seed{N}.bin ("EDT1", 25 B/entry), saved
  after every break/place, loaded at startup.
- Salt lakes tint mineral-pale on mesh and blocks. Gotcha discovered
  during verification: the biggest salt lake sits on a koppen-29 icy
  plateau, so it renders as ICE (the frozen ramp precedes the salt ramp)
  - correct, but pick lake 641/873 to see the salt look.

### Verification
p6-* captures in viewer/interchange; physim green (21/14 cave breaches,
1/5 solid trunks on the two transects); golden noise tests pass; 10 s
live-window smoke with the sky pipeline. Note for the family: walk mode +
pointer lock + gravity + a sunset over a great river is finally A GAME.

## The night-sky bug and the torch (2026-07-07, plus repo goes public)

User verification of Phase 6 came back with a fresh batch: the sunset sky
went BLACK when descending under ~8 m; walking face-first into a tree let
you see inside the trunk; towers turned gray (and in forests first tried
to become trees); placing a block mid-tower pushed the tower up instead of
building beside it; and a request — since caves are dark now, give the
player a torch.

### The black sky: two screenshots, three meters apart
The user's screenshot pair (alt 9 m: perfect sunset; alt 6 m: void) was
the whole diagnosis. Nothing in the sky math depends on altitude that
steeply — but the projection does: near = alt * 0.2. fs_sky unprojected
the FAR plane through the f32-cast inverse view-projection, and the two
matrix entries that sum to p.w ≈ 1/far (~3.9e-5) sit near ±1/(2*near).
At near = 1.2 mm those entries are ~416, where one f32 ULP is 3.05e-5 —
the true difference drowns in rounding, p.w comes out zero or negative,
every sky ray flips, and the below-horizon fade blacks the frame.
Fix: reversed-Z with an infinite far plane (perspective_infinite_reverse_
rh; Greater depth compare; clear 0; sky triangle at z=0), and the sky
unprojects the NEAR plane, whose inverse-VP column is numerically clean.
Verified at 6 m/9 m (sunset restored), eye height, 9 km, and 3000 km.

### Seeing inside trunks was the near plane too
With near floored at 0.8 m, any wall closer than that got clipped open —
trunks are one column wide, so walking into one put the near plane past
its skin. Reversed-Z makes depth precision distance-uniform, so the near
floor dropped to ~14 cm at eye height (corner rays reach 1.65x near ≈
24 cm), and the walker got a 0.35-block body radius: 8 probe columns ring
every horizontal step, rejecting walls above step height and tight
ceilings. You now stop ~a third of a block short of any wall.

### Towers: anchor generation to the natural ground
ColCtx now carries ground0 (pre-edit terrain top) alongside ground.
Strata, steepness->rock, the beach rule, and the cave band all anchor to
ground0, so a tower on grass is a dirt tower with a grass cap instead of
slope-triggered gray stone; digging still exposes dirt-then-stone (and
can genuinely open into the cave band, which is a feature). Any edited
column loses its tree — the "tower wants to become a tree" was the
hash-planted tree riding the edited ground height; now building on (or
digging) a tree's column chops it. Tree relief probes compare ground0 so
a tower doesn't shake down the neighbors' trees either.

### Face-aware placing
raycast_column returns (hit, last-air-column); E edits the air column
(build against the face you aim at), Q still breaks the hit column's top.
Aiming down at a top face grows that column as before. Column-edit model
caveat, now documented: a placed block always lands on its column's top.

### The torch
Cave darkness stopped being baked into chunk vertex colors: blocks carry
the dim factor in the unused water-attribute alpha and the shader applies
it, so a warm light pool around the camera (reach ~10 m, quadratic
falloff, gated by (1 - dim)) restores color where — and only where — the
world is dark. Daylight terrain is pixel-identical. Verified with an
injected test dig: a 2x2, 10-deep shaft at night shows warm-lit walls
fading with depth where the user's original pit shot was pitch black.
(Test edits were appended to a backup of the real edits file at an exact
column found with the new examples/colof.rs, then restored.)

### Repo goes public
- LICENSE: MIT (root), license fields in Cargo.toml, license section in
  the root README. Chosen deliberately for the family's wishes: the game
  stays free forever, and MIT contributions can flow into a possible
  future commercial edition without CLA paperwork (GPL would entangle
  that as soon as an outside PR merged; Cubyz was design reference only,
  no code copied, so no GPL obligation exists).
- Root README reframed for the whole project with a from-scratch
  bootstrap (rustup + Python + pip deps, generate res 7, bake both,
  cargo run) so the son can verify on a clean machine.
- .gitignore: generated data (output/, baked faces, meta.json, edits) and
  viewer/interchange/ stay out; initial commit made for the user to push
  to github.com/dualmindblade/triangulum.
- Headless --capture now loads saved edits, so screenshots match the
  played world (the dig-site repro capture was flat grass without them).

### Verification
fix-* captures in viewer/interchange (sunset 6 m/9 m, towers-day: dirt
towers with grass caps + correct shaft strata, shaft-torch at night,
forest at eye height, reversed-Z sweep at 0.3/9/3000 km vs the p6
baselines); physim green — the 400-column taiga transect matches
yesterday's output exactly (14 breached, 5 trunks); golden noise tests
pass; 12 s live-window smoke with the per-frame collision probes.

### Lesson
A coordinate-stamped screenshot PAIR is worth more than any log: "works
at 9 m, black at 6 m" pointed straight at the one quantity that changes
that fast — the altitude-scaled near plane — and the fix (reversed-Z)
retired a whole class of future depth bugs at once.

## Geomorphing, and the pop that mostly wasn't (2026-07-07, Phase 7a)

New workflow from the user: feature branches + merges for major features,
push to the remote each session. This one happened on feature/geomorphing.
Also from the user: the son is Andrew, the design credit is his.

### The feature
CDLOD-style geomorphing on the tile mesh. Every vertex carries two extras:
the height its PARENT level renders at this spot (parent vertices sit on
the child's even lattice, odd ones bilerp — heights are pure functions of
(u, v, octave budget), so when budgets match the delta is exactly zero and
the extra sampling pass is skipped), and the wetness the parent level
paints (same sample, doubled spacing — the painted river thread is widened
to the vertex spacing, so its width is level-dependent and pops at every
split). The vertex shader slides position radially and lerps wetness over
a distance band derived from the selection threshold tau:
  tile of size S is selected while center dist is in [S/tau, 2S/tau);
  morph must be complete at every vertex before any swap ->
  end <= (2/tau - sqrt2)*S, and still absent where children hand off ->
  start >= (1/tau + sqrt2/2)*S. With tau = 0.35: band [3.61, 4.26]*S.
Cross-level tile edges agree for free (the finer neighbor is fully
morphed exactly where the coarser one is unmorphed) — which is also why
skirts stay sub-meter. Radial direction comes from a new planet-center
global (camera-relative space has no origin for it otherwise).

### The measurement (the interesting part)
Built examples/popdiff: fly a fixed line, render every frame headless,
report frame-to-frame pixel diffs (mean + p99.9), and — decisive — print
which tiles the LOD selection swapped each frame. Findings, honestly:
- At 6 km cruise, SWAP FRAMES ARE PIXEL-INDISTINGUISHABLE FROM QUIET
  FRAMES even with morphing off (p99.9 ~30/255 both, identical medians).
  The architecture pre-solved most of the problem: each level adds one
  band-limited octave whose wavelength is 2.5x the vertex spacing, and the
  SSE threshold swaps tiles exactly where that detail is ~1 px.
- The visible artifact I chased across three instrumented flights (a
  bright double line in the diff heatmaps) turned out to be the painted
  river's edges under camera parallax, not a LOD event.
- At 2 km, EVERYTHING drowns under voxel-patch churn (the chunk ring
  recenters every frame) — p99.9 saturated at 82 in both modes. That, not
  mesh LOD, is the real frame-to-frame noise at low altitude; queued as a
  Phase 7 item.
So geomorphing here is insurance, not rescue: it converts near-
seamlessness into a guarantee, and it genuinely retires the river-paint
width pop (the one swap artifact big enough to see: ~16 px -> 8 px lines).
Verified correct by construction tests instead of pop hunting:
TRI_FORCE_MORPH renders every tile as its parent geometry — the world
stays coherent, no spikes, no cracks (sign/scale/direction proven);
TRI_NO_MORPH gives the raw baseline. Golden tests green; live smoke green.

### Patch boundary
The voxel patch ended on a floating one-block cliff (blocks ride ~1.6 m
proud of the mesh so it can't poke between block tops). Rim blocks now
sink flush over the outer ~15% of the patch radius, in the vertex shader
(the hole disc and lift are already in the globals; cached chunk meshes
never rebuild for it). The patch edge reads as a feathered shoreline —
gm-rim-sink.png vs fix-rz-valley.png shows the cliff gone.

### Housekeeping
Killed a stale viewer instance (running since 12:29 AM) to relink the
binary — relaunch to pick up the branch. Vertex is now 60 bytes (+8).
capture/popdiff share the renderer, so all evidence is same-code.

### Lesson
Measure the pop before building the anti-pop. The instrument (popdiff +
swap logging) cost 100 lines and reframed the whole feature: the scary
part was already solved by the octave-band design; the actual visible
pops were in the PAINT, which no geometry morph would ever have fixed.

## Night sky: stars and the limb (2026-07-07, Phase 7b)

Two sky-pass additions on feature/night-sky:
- Stars: view direction hashed into a sparse cube lattice of cells, each
  holding one jittered star with hashed brightness and a cool-to-warm
  tint. Purely directional (fixed to the sky), and they dim against the
  sky's own luminance — so they own the night and open space, and
  daylight hides them. No time uniform yet, so no twinkle; fine.
- Limb glow: from orbit, rays grazing past the planet get an atmosphere
  term by closest approach to the planet center (the camera-relative
  center global from the geomorph work paid off immediately): a thin
  blue shell hugging the surface radius, modulated by the sunlit side,
  fading in only as the camera's own atmosphere fades out.
Verified: ns-night-ground.png (starfield over the dark valley),
ns-orbit-limb.png (blue rim on the lit limb, stars behind, dark side
fades), ns-day-regress.png (no stars in daylight). Golden test green.

## Torches you can leave behind (2026-07-07, Phase 7c)

R toggles a torch on the block you aim at (same face-aware raycast as
E). A torch is two things sharing one persisted set (TRC1 file, 17-byte
entries, per seed):
- geometry: crossed thin quads on the column's walkable top, wood below,
  flame above. The flame rides a new emissive convention — chunk vertices
  with dim > 1.5 skip sun and cave darkness entirely in the shader.
- light: the renderer ranks all torches by distance each frame (cheap
  direction math first, terrain samples only for the winners) and ships
  the nearest 16 as camera-relative point lights in the globals. The
  fragment shader adds a warm quadratic pool (reach ~11 m) on blocks and
  mesh tiles alike, scaled by base albedo so it restores true colors in
  cave darkness rather than washing gray.
Torches ride column edits automatically (their height is re-derived from
top_solid at mesh time), and headless captures load them like edits.
Verified with three injected torches at the tower-test spot:
torch-night.png (overlapping warm pools under a star field — the game's
first cozy screenshot) and torch-day.png (subtle posts, faint glow).
Test file removed afterwards; live smoke green. Next candy on the
Phase 7 list: day/night cycle, flame flicker (needs a time uniform),
river aliasing, chunk-churn.

## The planet turns (2026-07-07, Phase 7d)

Day/night cycle on feature/day-night. The model is the physical one: the
sun is fixed in space and the planet rotates about its axis, so local
time depends on your longitude, flying east fast-forwards the day, and
from orbit the terminator wraps the globe. Default 20-minute day
(--day-len N seconds; 0 = the old sun-follows-camera mode; --sun-lat/lon
still pins exactly, so every repro command in the README stays valid).
The cycle starts at mid-morning at the spawn longitude — captures
without sun args now get pleasant angled light instead of noon-overhead.
A time uniform (misc.y) rode along and bought torch flame flicker: the
emissive quads breathe on a position-hashed phase and the point-light
intensities pulse per torch on the CPU.
Verification note for future me: a headless --capture draws ONE frame at
t~0.2s, so stills cannot show the cycle; popdiff grew a DAY_LEN arg and a
static-camera run at --day-len 60 shows every frame differing as the sun
sweeps (~1 deg/frame at cached-tile speed) — the lighting animates.
dn-morning.png (soft morning over the river valley), dn-terminator.png
(globe with day/night edge from 15,000 km), dn-legacy.png (old mode
unchanged).

## Planetgen gets fast (2026-07-07, feature/planetgen-speed)

The question that started it: could player-generated planets be viable?
Measured first (fresh res-6 run + cProfile): climate was 79% of the wall
clock — thousands of advection/diffusion passes over the (n, 6) neighbor
arrays, numpy-vectorized but CPU-bound — with render at 13% and the
hydrology erosion loop at 6% (its priority-flood heap and downstream
accumulation are pure Python).

Two optional accelerators, auto-detected, with plain numpy as the
always-working canonical path:

- numba on the two hydrology loops. The compiled priority flood replays
  heapq's (key, index) tie-breaking exactly, so both cores are
  BIT-IDENTICAL to the Python fallbacks (asserted in tests/accel_test.py)
  at 21x / 39x. Hydrology at res 6: 15.9 s -> 5.0 s.
- CuPy for the whole climate stage (cfg.gpu / --gpu). The grid operators
  now run on whichever device their input lives on (grid.arrays(gpu)
  mirrors the constant arrays to the GPU as float32 — consumer cards run
  fp64 at 1/32 rate). Arrays stay device-resident across the stage;
  bincount-flavored helpers (zonal_mean, itcz binning, insolation) stay
  on the host. Two classes of landmine found and fixed:
  * numpy f64 SCALARS (np.radians(-58.0), _q_sat(30.0)) are "strong"
    under NEP 50 and silently promote a float32 GPU pipeline to float64
    — wrapped in float() at four sites;
  * builtins.max() on a device scalar forces a GPU->CPU sync — the
    profiler showed 5,891 of them (2.5 s); replaced with np.maximum,
    and advect's host-side vmax branch became a device-side clamp
    (exact for all real inputs, zero transport when the field is still).
  First GPU run pays ~50 s of one-time kernel JIT (disk-cached forever).

Numbers at res 6 (41k cells, seed 5, warm kernels, through biomes):
  CPU 236 s -> GPU 40.6 s (climate1 115 -> 14.8 s, climate2 109 -> 13.5 s).
The CPU path is byte-identical before/after the refactor (asserted on the
stage pickle), and the GPU planet came out IDENTICAL where it counts:
elevation corr 1.000000 (cm-scale drift), ocean mask 100%, rivers and
lakes jaccard 1.000, koppen 100%. The float32 perturbations (~1e-4 degC)
die in the contractive erosion loop instead of compounding. Still: CPU
remains the documented same-seed-same-planet reference, since cross-
hardware bit-identity on GPU is not guaranteed.

Extrapolation to res 8 (16x cells, fixed iteration counts): climate
~8 min -> well under a minute each pass; the RENDER stage (~10 min of
matplotlib) becomes the biggest line item — irrelevant for in-game
generation, which only needs planet_data.npz. Estimated full res-8 with
maps: ~15 min vs ~90; without maps: single-digit minutes. The live res-8
proof run (seed 7) was stopped externally mid-run — the user was playing
the game on the same GPU, which is exactly the contention it warns
about; rerun anytime: python -m planetgen --seed 7 --res 8 --gpu.
--record works under --gpu (frames hop to the host); tests/accel_test.py
guards both accelerators for future refactors.

Strategic note stands: this makes ITERATION fast for us; shipping player
generation still points at a Rust port (chronicle as loading screen),
stage by stage with golden tests, climate first.

## The patch grows, the rivers thin (2026-07-07, Phase 7d/e)

Two branches from the user's asks: kill the recenter churn, make the
block disc bigger ("configurable or something — I think it would improve
the look and feel quite a bit"), and fix the confluence blobs.

### Patch streaming (feature/patch-streaming)
Chunk builds left the frame loop: missing chunks spawn onto background
rayon tasks (nearest first) and land whenever they finish; every frame
renders whatever is built. The heightfield hole now opens only over
ACTUALLY BUILT coverage — teleport somewhere fresh and you see mesh
terrain with blocks streaming outward through it, never a hole to the
sky. The rim-sink follows the selected radius (a new misc slot), so the
patch edge stays a feathered shoreline even mid-stream. Planet rides an
Arc into the workers; in-flight builds orphaned by an edit are dropped
via the pending-set guard and rebuilt fresh. --patch N (0.3-2.0) scales
the disc; the difference at 2.0 is dramatic (ps-patch1 vs ps-patch2:
stepped-bank lakes and shrub texture out to the middle distance where
there was flat paint). Found the hard way (wgpu OOM mid-verification,
with the user's own game holding half the card): fast flight at big
patches accumulates chunk buffers faster than age-based eviction retires
them — the chunk cache now has a hard 1.5 GB VRAM budget with
newest-first retention. Headless captures block until streaming settles,
so screenshots stay complete frames.

### River LOD paint (feature/river-lod-paint)
The painted river corridor is widened to one vertex spacing so distant
threads stay continuous — and was painted at FULL strength no matter how
little of that corridor the real channel fills. That's the confluence
blob: two full-opacity corridors meeting at an acute angle fill the
wedge. Paint now carries sqrt(hw / painted_width) coverage: a 30 m
tributary across an 800 m corridor is a faint thread, the great river
keeps its weight, and thin rivers fade with distance instead of
shattering (rv-delta before/after). Ponds' feathered wetness fades past
its 0.28 km feather scale by the same rule. Verification surprise: the
angular blue polygons in the valley views did not change AT ALL —
pixel-identical — because they are LAKE-CELL FOOTPRINTS (geometry, not
paint). New Phase 7 item: lakes need a soft distance-to-shore signal.

### Verification note
The user was playing the whole session (their viewer holds the exe lock
and half the VRAM), so everything was verified through the example
binaries: the streaming machinery ran headless across 40-frame flights
(the capture path drains the same channel), the bin passes cargo check,
and nothing on either branch touches noise (golden test unaffected).
Their next `cargo run --release` builds and launches the merged code.

## The play harness (2026-07-07, Phase 8 begins: AI verification)

Discussed with the user how weaker models can verify work end-to-end
when stills are so much weaker than a human just playing. Decisions:
sidecar state files beat burning coordinates into pixels (models read
text perfectly, pixels expensively); a minimap is a v2 nicety, not the
navigation mechanism; navigation is ABSOLUTE-first (teleport + look) —
dead-reckoning is exactly where weak models lose the plot.

Built: src/player.rs extracts the player simulation (walk/fly physics,
jump/swim, teleport, block/torch edits) from the window app — one
implementation, two drivers. examples/play.rs interprets .play scripts
(teleport/look/mode/hold/tap/wait/shot/state/sun/log) at a fixed 60 Hz
timestep, headless, in a clean world: every run leaves frames, JSON
state sidecars (pose + physics + the terrain sample under your feet),
and a transcript. Deterministic by construction.

The harness earned its keep on its SECOND run: the valley-walk script
built a 3-block tower and then could not move in ANY direction. Real
bug (shipped in the trunk-collision work): the body-radius ring blocked
every move once a wall stood inside 0.35 blocks — and placing blocks
next to yourself creates that state instantly, then no move can ever
increase clearance. Deadlock. Fix in player.rs: a violated probe only
blocks movement TOWARD it (dot(heading, probe) > 0.1) — escape and
wall-sliding work, the approach standoff holds. Andrew has almost
certainly hit this in play; the harness found it in minutes.

Also learned: transcripts need 6-decimal coordinates (at 4 decimals a
5 m walk is invisible — the tool was gaslighting its own operator), and
script files must survive Windows BOMs. Verified: valley-walk end to
end (tower, torch, block-by-own-wall with state proof, escape), goldens
green, physim exact, live smoke green after the physics extraction.

### The codex experiment (same session)

Handed the harness to an outside model cold: codex CLI (GPT-5.5,
reasoning xhigh, workspace-write sandbox) with a one-page mission — "find
something obviously wrong, here's the harness doc, deliver FINDINGS.md
with a repro". It authored its own probe script (shoreline scans of the
lake archipelago and the inland sea, sun pinned for controlled lighting,
eight aimed shots), read the state sidecars, cross-checked the planet
dataset (looked up the nearest cells in planet_data.npz to confirm
lake_id 526/413, not ocean), and filed a correct, reproducible finding:
LAKE FOOTPRINTS RENDER AS GRID-ALIGNED POLYGON CELLS at coarse LOD —
independently converging on the exact artifact diagnosed earlier today
during the river-paint work (already on the Phase 7 list). It also
correctly reported what it checked that was NOT wrong (no ocean-mask
misclassification, no patch-seam issue at low altitude). ~15 minutes,
no source modifications, and the freeze the user remembered never
happened (a log-staleness watchdog was armed regardless).

Verdict: end-to-end AI verification through the harness WORKS. Two
independent AI investigations through different tools now agree on the
top cosmetic bug — soft lake shorelines just moved up the queue.
Operational notes for repeat runs: launch codex via cmd (the Git Bash
npm shim is broken), model id is gpt-5.5 on this account (not
gpt-5.5-codex), missions in viewer/interchange/codex/.

## Phase 8b — dual code review + play-harness discovery + delegated fixes (2026-07-07)

A full pass: two independent code reviews (Claude + codex GPT-5.5 xhigh),
a play-harness bug hunt, and a delegated fix round. Verified work committed
to main and pushed as it landed.

### The two code reviews (independent, converged)

Claude (me) read the whole viewer; codex reviewed it cold via the harness's
Start-Process launch (see the freeze note below). Consolidated findings I
judged worth fixing:

* **[codex, high] Stale in-flight chunk mesh wins the rebuild race.** A chunk
  build carried only its ChunkKey. On a block/torch edit, invalidate removed
  the pending key, the next frame re-queued a fresh build, and the OLD build's
  result (stale edits) was accepted on arrival because the key was pending
  again — dropping the correct rebuild. Edits could render stale until the
  next invalidation. FIX (me, renderer.rs): per-request monotonic epoch;
  drain/capture accept a result only if its epoch still matches.
* **[codex, med] chunks_touching_column missed diagonal + cross-face
  consumers.** build_chunk reads a TREE_MARGIN ghost ring and canopies reach
  into it, so a corner edit must invalidate the diagonal chunk and a face-edge
  edit the neighbor-face chunk. FIX (codex, voxel.rs): scan the ±TREE_MARGIN
  neighborhood, map each column to its canonical owning chunk, dedup.
* **[codex == my #3, med] Face-edge tree anchors hashed with wrapped
  current-face indices** — canopies decided differently on the two sides of a
  cube-face edge (and vs tree_here physics). Both reviews found this
  independently. FIX (codex, voxel.rs): canonical_column() identity map; hash
  anchors + tree_cells rnd with the canonical (face, ci, cj).
* **[me, med] Near-field chunks starved by the MAX_TILES draw cap.** Tiles
  were enumerated before chunks and truncated at 4096, so an over-cap frame
  dropped the near CHUNKS (see-through hole under the already-cut heightfield).
  Measured 3.7k draws at --patch 2.0 low altitude — real headroom risk. FIX
  (me): draw chunks first, MAX_TILES 4096 -> 8192.
* **[me, med] evict() could drop a chunk queued to draw this frame** (the pass
  indexes chunk_cache[k] unconditionally) if the visible set alone exceeded
  the 1.5 GB budget -> panic. FIX (me): never evict last_used == frame_counter.

Divergence was as useful as convergence: codex owned the chunk-edit /
threading bugs, I owned the draw-budget / runtime bugs and the play-harness
find. Lower-priority items logged but deferred: near-unwalkable steep peaks
(1-block step-up vs jagged ridges) and dead-flat 2 m coastal sand plains
(h_floor clamp) — both arguably by-design; noted for a design decision.

### Play-harness discovery: underwater patch-boundary water void (Claude)

Sinking into the inland sea and sweeping the view revealed a near-black
see-through slit at the water surface where the voxel patch meets the far
mesh — the sky visible THROUGH filled water (obviously wrong, non-aesthetic).
Isolated with the harness's determinism: world-locked (yaw 0/180 only), and
GONE at --patch 2.0 (boundary pushed out) -> it's the patch boundary over
water. Root cause: the heightfield hole cuts the far mesh's sea/lake WATER
surface, but block water and mesh water are the same surface, so any perimeter
crack (rim-sink, T-junction, quantization) shows sky. FIX (me): a per-vertex
wflag marks sea/lake water-surface verts; fs_main skips the hole discard for
them, so the mesh water plane backs the patch and any crack shows water, not
void. Verified: void gone at all yaws, previously-clean views + river delta
byte-identical (rivers wflag=0), above-water sea shows no z-fighting.

### Delegation (the point of the exercise)

* Me: the underwater void (rendering, interesting) + renderer safety (A/D/E,
  incl. the high-severity threading race).
* codex GPT-5.5 (worktree, isolated): the voxel.rs chunk-edit correctness
  (B/C) — clean canonical_column() refactor, reviewed + compile-verified by me
  before merge (codex didn't build).
* Branches: fix/underwater-patch-water-void, fix/render-safety,
  fix/chunk-edit-correctness — each verified, merged to main, pushed.

### Operational: codex logging + freeze (IMPORTANT correction)

The documented `cmd /c "codex exec ... > run.log 2>&1"` launch does NOT work
from this agent harness — bash->cmd file redirection silently creates no file
(a trivial `cmd /c "echo x > f"` also fails here), and the first attempt hit
the known silent freeze (378 CPU-s then idle, zero output). ROBUST launch:
PowerShell `Start-Process cmd.exe -RedirectStandardInput MISSION.md
-RedirectStandardOutput run.log -RedirectStandardError run.err -NoNewWindow
-PassThru`. codex streams reasoning to STDERR (run.err) and the final answer
to stdout; watch run.err for liveness and sample process CPU (flat CPU + no
new stderr = frozen) rather than log-file growth.

## Phase 8c — codex 4-round play-hunt, fixes, and making discovery systematic (2026-07-07)

The mandate: run the play harness as a bug factory (me + codex), fix what it
finds, and — the real task — **critique the verification method and change it
so automated discovery finds more**. This entry is that critique, backed by
what the session actually produced.

### The harness is a genuine bug factory

Across two independent AI operators, focused play runs surfaced real,
reproducible, NON-aesthetic bugs at a high hit rate. codex's four rounds:
* **R1** placing a block while looking straight down (walk mode) embedded the
  head in the placed block — a black starfield void through solid rock.
* **R2** fly mode never set `underwater`, so flying below a river surface
  rendered as an above-water view.
* **R3** frozen lakes render as a pale ice sheet but used liquid swim physics
  — the player sank through visible ice and swam.
* **R4** the underwater tint stuck `true` after teleporting above a lake until
  the next fly tick.

The common thread — and the single most important design choice in the
harness — is that **every find was confirmed by the state JSON, not by "looks
weird."** `ceiling_above_km` below the eye; `underwater` disagreeing with
`water_surface_km`; feet ≠ support. The sidecar turns a suspicion into a
provable inconsistency a weak model can assert on. Weak models read text
perfectly and pixels expensively — betting the harness on machine-checkable
ground truth is why it works.

### The fixes (all verified, one merge to main)

* **R1** — `edit_block` now rejects a placement that would intersect the
  player's own body (own column + new block span overlapping [feet, head]).
  The consistent voxel rule: you can't place a block inside yourself.
  Pillar-jumping (feet already above the new block) still places.
* **R2+R4** collapsed to one root cause: `underwater` was set ad-hoc per
  branch (fly hard-coded it false; teleport left it stale). Now
  `refresh_underwater()` derives it from eye-vs-water in ONE place, called
  after every pose/mode change.
* **R3** — `ColCtx::frozen_ice()`: a below-freezing water column is a solid
  ice sheet in `support_below_km`, and `water_surface_km` hides it from
  wading physics. Delegated pattern check to myself; verified on the exact
  codex coordinate.

Delegation this round: an easy, well-specified task (consolidate the four
repros into `scripts/physics-regressions.play`, run it green) went to Sonnet
— which cleanly handled it AND caught a real subtlety (the combined script
needs an explicit `mode fly` before R2 because R1 leaves it in walk).

### Where the method was weak, and what I SHIPPED

1. **Runs were passive** — every run "succeeded"; a human had to eyeball each
   frame + JSON. SHIPPED earlier this session: the `assert FIELD OP VALUE`
   command (fail = non-zero exit) and `invariant-survey.play`. This is the
   biggest lever: it converts discovery into a growing regression suite, and
   every fixed bug now leaves an asserting script behind
   (`physics-regressions.play`).
2. **Discovery was hand-authored** — both operators had to *think of* where to
   look, and gravitated to water/coasts. SHIPPED the real upgrade:
   `scripts/gen_survey.py`. It samples MANY real feature cells straight from
   `planet_data.npz` (interior cells only, so a point feature can't miss its
   edge after the icosphere→cube resample) and emits one asserting probe per
   cell, per class: inland land + peaks must be walkable (grounded), frozen
   lakes + sea ice must be solid dry ice, open water must immerse. This
   **generalizes every single-point find into a whole-class gate** — R3 is no
   longer "one frozen lake"; it's "every frozen lake." 45 probes, deterministic
   and diffable, run in seconds.
3. **The sweep already paid for itself.** On its first run it flagged an
   "ocean" cell at 84°N reading as dry — not a bug: my frozen-water fix
   correctly treats a −15 °C polar sea as **sea ice** you walk on. The sweep
   validated that R3's fix generalizes beyond lakes, and I split sea ice into
   its own gated class. That is discovery finding more with zero human
   authoring of the probe.
4. **A build trap cost real time.** `cargo build --release` does NOT build
   examples, so I verified R2/R3/R4 against a STALE `play.exe` and briefly
   misread three working fixes as broken. Rebuilt with
   `--example play` and all passed. Now documented loudly in the harness
   README — this is exactly the kind of silent failure that makes automated
   verification lie.

### On the overall process (reviews + hunt + delegated fixes)

* Two INDEPENDENT reviews earned their cost: convergence (both found the
  face-edge tree-identity bug) raised confidence; divergence widened coverage.
* Delegation works with clear scope + a verification gate the delegate must
  hit (Sonnet: run it green; codex earlier: fix in a worktree, I compile-check
  before merge — the delegate didn't run the compiler).
* The division of labor is right: the harness owns OBJECTIVE consistency
  (physics/geometry/rendering contradictions), humans own TASTE (is the flat
  2 m coast a bug or a style choice?). The harness deliberately can't and
  shouldn't adjudicate aesthetics.

### Net change to the method

The front line of discovery is now a **generated, self-checking survey** that
fails loudly, seeded from the source planet data — not a human flying around
looking at stills. Next levers (recommended, not yet built): fold the survey
into a session-start gate; add a `nearest <feature>` probe or a shipped
feature-coordinate manifest (scripts still can't branch on state); and widen
classes to cube-face edges and cave interiors, the two places both operators
under-visited.

## Phase 8d — recommendations shipped, the moon, and a second codex hunt (2026-07-07)

Acting on Phase 8c's own recommendations, then running the loop again.

### The three recommendations, shipped

* **Cube-face-edge sweep coverage** — `gen_survey.py` now derives seam
  coordinates from the cube geometry (mirrored `FACES`), maps each to its
  nearest dry-land cell, and asserts grounded. 61 probes / 7 classes. The
  ±45°/±135° meridian seams all hold — the `canonical_column` cross-face work
  is solid.
* **Session gate** — `verify.sh` rebuilds the harness (examples included!),
  regenerates the sweep, and runs all three suites, non-zero on any failure.
* **Feature-coordinate query** — `where.py` (nearest-feature lookup +
  `feature-coords.md` manifest); class defs extracted to a shared
  `feature_masks()` so the query and the sweep can't drift.

### Roadmap: the moon & moonlit nights (Phase 7f)

A phase-lit moon opposite the sun (real terminator reconstructed from the disc
normal, earthshine, tight halo), tied to the sun so pinned-sun captures stay
reproducible; plus a cool directional moonlight lift so night terrain reads as
moonlit instead of near-black. Daytime untouched (gated by `1-day`). QA'd
across night ground, twilight, and orbit.

### codex round-5 hunt (4 rounds) + fixes

Ran codex again on the improved harness, with its OWN copy of the play binary
so my parallel renderer rebuilds (the moon) couldn't corrupt its runs, and a
mission steered at what the assert-sweep can't see (visual/cave/edit/night).
All four found, all fixed, verified, merged:

* **R5-1** breaking the block underfoot left `grounded=true` hovering above
  the new support (derived state not resynced after an edit — the R4 class).
  `refresh_after_edit` (grounded + underwater) after every break/place.
* **R5-2** solid sea ice rendered as a featureless flat plane. `frost_color`
  snow-dusts frozen sea/lake with an LOD-stable position drift (mesh + voxel).
* **R5-3** couldn't step out of a dug 1-block hole: the body-radius probe
  measured headroom above the LOW feet, so a steppable rim's own step block
  read as a zero-headroom wall. Measure headroom at the probe's step-up level
  instead — also repairs 1-block *ledge* step-up generally.
* **R5-4** dug shaft floors rendered as grass underground: `mat_at` measured
  strata depth from the dug-down top, not the natural `ground0`, so the floor
  computed depth 0 → grass. Measure from `ground0` → dirt then stone.

### RETRACTED: "the harness screenshots show only the MESH" was WRONG

An earlier draft of this log claimed the harness renders only the far mesh, not
the voxel near-field. **That was a misread and is retracted.** `capture()`
already blocks until `chunk_pending` drains, then redraws — it renders
fully-built voxel chunks. Proof (2026-07-07): instrumenting the hole showed a
level-view capture with r_km=0.4038 km (a 404 m hole), 468 chunks drawn, 0
pending; a magenta-paint debug of every heightfield fragment lit up ONLY a far
band past the hole — the entire foreground was voxel-chunk geometry.

The actual trap: **flat-savanna voxel-chunk tops look smooth and mesh-like.**
Each column's top is a single 1 m quad; coplanar neighbours read as a faceted
sheet, so gentle biomes masquerade as the LOD mesh. This false "it's only mesh"
read was reached twice (a straight-down look and a flat savanna) before the
magenta diagnostic settled it. Lessons that survive: (1) the `frost_color`
snow-dusting still needs both a mesh and a voxel path (the mesh shows past the
patch); (2) before ever concluding "mesh, not voxels" from a harness shot, run
the magenta paint (`if rel_flag.w>0.5 { return magenta }`) — the mesh is
whatever turns magenta, everything else is voxels. There is no known mesh-only
or steep-pitch rendering bug.

### Method notes that held up

* Binary isolation for a delegated agent working *while* you edit shared code:
  give it a built copy, tell it not to rebuild. codex stayed on its snapshot
  through all my renderer churn.
* Instrument, don't theorize: R5-3's block resisted static analysis through
  many passes; one `TRI_DBG` eprintln in the collision path named the exact
  failing probe (`ceil=0.0`) in one run.
* Edits persist within a play run — two digging regressions at the same column
  contaminate each other; order them or separate the columns.

## Phase 8e - the water-wall family falls to a census (2026-07-08, Fable overnight)

The humans kept finding "big and obvious" water bugs by flying around
(stepped river cliffs, a giant sunken pie slice in what looked like ocean,
grey striped curtains at canyon heads) faster than the harness did. This
session closed that gap: build the tool that finds the whole class by
exhaustion, then fix the class.

### The census (examples/census.rs)

Sweeps every river segment (cross-channel transects) and lake cell (radial
spokes) through terrain::sample, BISECTING every adjacent disagreement about
water until it either dissolves (smooth ramp / real levee - not a bug) or
survives at <=12 m separation (a cliff a player can stand under). Classes:
WALL (water >2 m above adjacent dry ground), JUMP (water-vs-water level
jump), SEAJUMP (vs the sea). Deduped into ~1.5 km sites, sorted by
magnitude, emitted as teleport commands. --at densely grids one site;
--probe dumps full Sample state for triage. ~104M samples, ~2 min with
rayon (gotcha: par_iter().step_by() serializes the pool - use filter).

BASELINE: 351,711 sites - 1,410,517 WALL / 288,174 JUMP / 130 SEAJUMP,
worst a 3.7 km wall of lake standing over a valley.
AFTER (all fixes): 115,248 sites - 262,542 WALL / 5,290 JUMP / 6 SEAJUMP.
JUMP -98.2%, SEAJUMP -95%, WALL -81%; the reported bend site went 668
findings -> 0 in 126k dense samples. The residual WALL population is
dominated by TWO knife-ridge mountain lakes (3282 m at -12.1 107.3, 3810 m
at 50.9 -28.1) whose 31 km cells overhang their flanks - ledgered as W-5
(needs a bake-level whole-lake fix; two per-sample gates were
census-measured to only move/worsen those walls and were REVERTED - the
census earning its keep as an honest referee against plausible fixes).

### One disease, four expressions

Every human-reported artifact traced to water surfaces that are not
hydraulically continuous:

1. bake_rivers levels were BED-anchored (min-relaxed to the deepest bed
   along each route): routes through dry pits carried km-negative levels ->
   bottomless slot gorges beside sea-level terrain; mouth segments lerped
   into ocean BATHYMETRY -> rivers reached the sea in sunken corridors
   walled in by water. NOW: levels are the hydraulic FILL SURFACE
   (level = max(own elev, receiver level) up from terminals; oceans pinned
   to 0; in-lake nodes pinned to the lake surface; re-anchor vs the fill).
2. The 586a828 lake flood (raw 3-radius disc) poured over dams. NOW:
   bounded by the sim rim ring (lake Voronoi + rim-cell territory at
   <=1.15r of a rim center); outlet channels own their water once clearly
   descended. lake-regressions.play caught the first too-tight version of
   this boundary (dried lake 414's pit again) and drove the final rule -
   the regression gate did its job.
3. Winner-take-all nearest-segment river levels cliffed along Voronoi
   bisectors at bends/confluences. NOW: levels blend across in-influence
   segments (closeness^2) - continuous by construction; cliffs become
   rapids in the overlap zone.
4. Ponds formed inside river floodplains AND on slopes, hanging raster-
   anchored flat surfaces up to 20 m above the downhill terrain. NOW: gated
   out of carved/valley ground, and a pool may never stand above its own
   column's natural pre-dig ground (water that would drain does not exist).

Verified: full verify.sh green (now 69 probes incl. liquid lakes, below);
water-walls.play re-shoots every reported site - the temperate bend is one
continuous river, the pie slice is a flat lake with a real island (the
"residual hole" probed as terrain 0.7 m ABOVE the level: a shoal, not a
water bug - ledgered V-2 for shore material), the canyon head is a clean
desert river reaching the sea, lake 414 unchanged. REBAKE REQUIRED
(bake_rivers.py) - rivers.bin level semantics changed.

### Parallel delegation (Opus 4.8 + codex GPT-5.5, isolated worktrees)

* Opus: terrace-ring shading fix - 69a78fa's quantized-height normals tilt
  a one-column ring at every terrace edge on gentle slopes (dark contour
  rings). Top normals now difference the CONTINUOUS h_km carried on ColCtx;
  edited/carved columns keep the quantized fallback. Baseline-vs-after at
  4 sites, verify green. (Worktree + own assets copy; zero collisions with
  the parallel water work.)
* codex: the survey's liquid-lake class was EMPTY (tmean>6C + interior()
  = 0 cells on a planet whose lakes are cold and tiny - median 1.7 C and
  3 cells). That is WHY this bug family was human-eyes-only. New class
  (lake & tmean>-4, cell-center sampling): 8 liquid-lake probes, all green
  post-fix. Also: every screenshot (P key AND --capture) now writes a JSON
  sidecar with pose + EFFECTIVE sun + day-cycle state, and
  scripts/repro_shot.py turns any shot (even legacy filename-only) into a
  .play script / --capture command - human reports are now one command from
  reproducible. (codex sandbox cannot commit in a linked worktree - the
  gitdir lives under the read-only main tree; its diff was reviewed and
  committed for it.)

### Method notes

* Instrument before theorizing held again: the first W-1 mechanism read
  (lake vs SEA) was wrong - rivers.bin inspection showed the ocean 119 km
  away; probe data found the real outlet-below-fill story in minutes.
* The census turns "fix verified at the reported site" into "fix verified
  at every site of the class, counted" - and it REJECTED two plausible
  fixes tonight by measuring them worse. That is the standard water/terrain
  fixes should meet from now on.
* PowerShell 5.1 traps that cost time tonight: embedded double quotes in
  native-exe args get mangled (keep commit messages quote-free), and a
  Get-Content/Set-Content round-trip mojibakes UTF-8 (use IO.File with
  UTF8Encoding(false) or dedicated file tools).

## Phase 8f - the shading saga ends at the normals; the rivers find their shores (2026-07-08, evening)

A day-long duet: Austin flying and photographing on a hash-verified build,
each report falsifying the previous explanation until only the truth was
left. Two long threads closed at their roots.

### The shading saga (seven layers, one root)

"Banding" turned out to be SEVEN stacked artifacts: fall-line stripes
(F-2), terrace rings (F-5), lee-face collapse (F-8), riser bake (F-11),
near-black dirt palette, low-moon inversion (flat night floor), and -
found when Austin asked "those two faces have the SAME orientation,
right?" - the root: side-face normals were POSITION-derived. The
out_n = normalize(da+db) - up*dot(...) formula left the face's offset from
the CHUNK CENTER (~28 m) rather than its orientation (~0.7 m), so every
side normal in the world swept radially away from its chunk's middle:
same-orientation faces lit by where they sat, patterns repeating on the
32-column chunk grid, twin shrubs bright vs black, erratic water teeth.
Fixed with true per-column horizontal axes at all three face sites
(terrain risers, water sides, tree faces). Then the last asymmetry: wall
faces got the missing half of the corner AO (tops darkened toward walls,
walls gave nothing back - creases now shade continuously around the fold).

Austin's method note, worth keeping: he annotated screenshots with arrows
and labels, counted nothing but refused every explanation that did not
cover his evidence, and was right four times in a row. The census and
sidecar repro pipeline turned each of his photos into a deterministic
experiment within minutes.

### The shoreline saga (clamps were treating a data disease)

The one-block river lip was STRUCTURAL: water was only assigned inside the
channel (riv_d < hw) while the bank blend carves banks ~1.2 m BELOW the
water level - every river ringed by dry land under its own waterline. Two
rendering clamps (4- then 8-neighbour) could only mask it. The real fix:
carved-below-the-waterline columns flood (same perch guard) - the
shoreline lands where the carved profile crosses the level, which rounds
flush by construction, and rivers now render at their TRUE carved width.
Voxel lip survey (census --lips, built this evening on Austin's
suggestion): raw findings -66%; remaining river entries are frozen shores
(walkable ice, exempt by design) and W-5 lake-seam residuals.

### Also landed today, same session

Photo-map teleport UI (egui + hand-rolled wgpu paint backend; markers/
list/preview, bulk delete to trash, opt-in time travel from sidecar
day-cycle time), build stamps in title + sidecars (option_env, never
fails the build), cave suppression near water tables (W-6 mitigation,
after Austin photographed his own morning prediction), dirt as dry loam,
sky-shaped night floor.

### Flooded caves (W-6 proper, feat/flooded-caves)

Austin's ask: "caves very near lakes and rivers will probably need to be
full of water!" The 0707a2f mitigation only SUPPRESSED caves near water to
kill the dry pit; this makes them real. The single-surface column model
(one `water` block) can't express "pool under solid rock", so ColCtx grows
one field: `cave_water`, a per-column LATERAL water table read from the
sample even on dry banks - the river's graph level (new `river_level_km`
on Sample) within a bank band, a lake's spill level within its shore band,
or sea level right on the coast. A carved cave cell at or below it is
water. A perch guard (`wt <= ground0`) forbids the table standing above the
column's own surface, so a dry column below the waterline never grows a
water wall.

Mesher: cave-water faces are their own pass - a free TOP surface only where
dry air sits above the pool (air pocket to dive from, or an open pit read
from above), SIDE faces only into a DRY cave passage, never against open sky
over lower ground (that was the W-1 water-wall shape). Cave darkness (the
depth dim carried in the water attr's alpha) applies, so a shallow pit pool
is day-lit and a deep flood goes dark until a torch reaches it; the global
underwater tint does the "I'm submerged" work. Physics: `water_surface_km`
gained an `at_km` arg and reports the sub-surface pool ONLY when the query
point is at/below it - so a bank walker over a flooded tunnel is NOT swimming
and a dry lake shore keeps `has_water == none` (the surveys still pass).
Cave water is always LIQUID (underground; never the walkable-ice path).

Verified at a river-bank pool (3.7259 63.0658, found with the new caveprobe
example): a walker drops in reading underwater + has_water, rests on solid
rock (no fall-through), swims back up and surfaces; digging a shaft down
through a 1-block rock cap floods it and drops the player through. The old
dry-pit coordinate 3.726 63.065 now reads as water pockets in the bank, not
a hole. scripts/flooded-caves.play (7th verify suite, all green) locks the
physics; flooded-caves-visual.play carries the frames. LIMITATION: the model
still can't show an air pocket a player must reach through a fully-submerged
passage - such a pocket reports submerged.

### Open, in recommended order

W-5 (knife-ridge lakes - bake-level whole-lake footprint; the census
holds the finish line), fresh discovery round against the now-clean
baseline (the big families were masking whatever is underneath), W-3
(rapids/waterfalls art), V-1 + texturing (the "better than Minecraft"
conversation - Andrew's call).

## Phase 8g — the Sol review triage (2026-07-09)

GPT-5.6 Sol (new strongest codex model, effort=max) ran the first full
independent review of the codebase: 16 findings, 11 major. Every one was
re-verified against code and runtime before action — ALL 16 CONFIRMED,
zero refuted, several with Sol's own runtime demonstrations. Calibration:
trust Sol's "certain" (the re-verification stays mandatory and found
nothing to correct). Triage: 6 findings fixed by Fable directly
(ice-solidity unification 0ec7b8d, NaN/persistence robustness d079d31,
water-wall family 072a512, photo restore 31acf19), 7 delegated to codex
GPT-5.5 as one sweep (c4f171b: deterministic render time with a
byte-identical double-run gate, Arc world snapshots, key auto-repeat,
assert semantics, preview staleness, transactional trash, caveprobe
seams), 1 delegated to Opus (bb16cba: cube-edge ocean-mask exactness with
a 12,360-pair property test), 2 backlogged with ledger entries (V-6 LOD
morph pop — TRANSITIONS program; T-1 film-scale census blind spot).

The headline was Sol's finding 1: the W-5 renderability cap bounds the
bake's blended raster, but the viewer's procedural relief digs beneath it
— census generalized one 143 m liquid wall to 683 lakes (114,421 wall
findings). Two capping strategies were implemented and MEASURED DEAD
(outside-dip caps: 648/678 lakes dry; two-sided caps: 628 dry — no level
choice survives territory boundaries crossing relief gorges). The shipped
fix changes the GROUND instead: shore aprons in the shared sample()
(floor at level, 3% bank fall-away, liquid-only), flood bounded to 2.6
lake-radii (killed phantom pools around far conduit rims), river
ownership extended to the whole carved gorge. Census: -90% findings,
median 21->6 m, max 267->76 m; residual = the merged mega-lake family,
ledgered as W-7 pointing at the same planetgen fix as W-5b. Method note
for the record: three successive "obvious" fixes at three sites each
turned out to be a DIFFERENT mechanism (union-of-frontiers distance,
coverage truncation, exemption-seam width) — the probe-first discipline
(lakeprobe.rs, new) beat pattern-matching every time.

Also: verify.sh counts SEVEN suites (Sol corrected our own docs), and
sea-ice solidity + under-ice ceilings are now locked in invariant-survey.

## Phase 8h — the pond wall family (2026-07-09, evening)

Austin field-tested the Sol-triage build and photographed an 18 m water
wall at a "lake" (-0.798 -67.941) — census triage showed it was a POND
(the noise-pond system), the third variant of the mask-ends-mid-relief
disease after lake territories and river gorges. The fix took SIX
census-gated iterations, four of them wrong in instructive ways:
unconditional interior flooding (79.5k pond walls + 141k jumps — ponds
blanket mountainsides), a fixed 30 m bench budget (razor-edge cliffs at
the budget boundary), an envelope-scaled bench without a water gate
(benched a lake-flooded dip through its own lake surface: +54 m pond
terrace ON a lake), and a 2%-grade gate alone (raster blends hide 55 m
escarpments at 1.8%). What landed: benches for interior dips (never
under existing water), pn-graded quadratic shore aprons (valley gate
included), and three spawn gates (env0 < 0.13, <2% coarse grade, never
inside lake flood territory). Every planet-census class ended BELOW its
morning baseline: sites 52.9k -> 20.1k, pond walls -68%, JUMP -61%,
liquid lake walls -64%. Since the pre-Sol baseline this morning: sites
109.7k -> 20.1k (-82%). The method note stands: every single hypothesis
this session that was shipped without a probe first turned out wrong;
every probe-first fix survived. Ponds visibly change for Andrew (gone
from crags, slopes, lake basins; interiors are shallow pools) — F-22.

## Phase 9 — living weather, W1 (2026-07-09, night)

Austin greenlit the weather thread: use the 12-month climate fields, real
gradations of sky and precipitation, landscape response, "some kind of
real-time sim if feasible", everything tunable, reproducibility intact,
zero contact with the transitions program. WEATHER.md is the contract;
Phase W1 shipped in one evening because the determinism work paid off
twice: render time is already the sim clock (F-20), so a weather system
built as a PURE FUNCTION of (seed, position, time) — climatology harmonics
from weather.bin + synoptic noise advected along the climatological wind —
is a real moving-fronts sim with no mutable state at all. The double-run
gate proves byte-identical frames WITH live weather; every suite stays
green untouched because W1 never enters sample().

Landed: scripts/bake_weather.py (harmonic fit: 0.63 C RMS residual vs a
9.35 C mean seasonal swing; Cartesian a/b coefficients because amp/phase
cannot be blended across texels), viewer/src/weather.rs (field + tuning
knobs + optional weather_tuning.json), cloud deck in fs_sky (integer-hash
value noise after float-hash fract() precision collapsed into diagonal
shards at big lattice coords), overcast dimming + sky murk, snow dusting
and rain darkening in the SHARED fragment shader (mesh and voxels get
identical weather by construction — the transitions boundary), instanced
rain/snow particles, `--weather off|live|C,P` CLI, `weather` play command,
weather state in photo sidecars, weather-visual.play demo. Reference
frames: wx_*.png in interchange (savanna storm, plateau deep winter,
rain streaks, snowfall).

## Phase 10 — transitions night: impostors + climate tint (2026-07-10)

Andrew's verdicts landed (TRANSITIONS.md top block): seamless-first with
a visibility knob, E greenlit, D gains the dual-range rule, palette
provisional until textures. Two features shipped the same night:

E v1 (Fable): forest impostors — mesh levels 11-12 grow crossed
billboard quads from the SAME placement lottery/species/trunk-height/
palette as voxel trees (tree_at refactored into shared parts); the hole
cut hands off at the patch rim automatically. Two-phase enumeration
(hash lottery on a strided lattice, terrain sample only on the ~4k/tile
budget survivors). Measured wrong on the way: bark-bottomed billboards
stack into brown bands via overlapping parallax (canopy-sea coloring
instead); species need silhouettes (taper) or the band reads as a picket
fence. Forests now hold from the rim to ~3-6 km, D's tint owns beyond.

D v1 (GPT-5.6 Sol, first public-day feature mission): planet::
climate_surface — continuous temp/precip ramps + smoothed Koppen hue
nudge + shared main-block category, consumed by BOTH shade_ground and
voxel surface_mat (grass/sand/snow tints and the snowline hash are now
literally one implementation). Cross-block ecotones dither per canonical
column over a 300 m band with a real gnomonic texel metric; same-block
pairs are pure gradient (Andrew's rule, verbatim). Sol ran every gate
unprompted — 7 suites, cargo test (+2 unit tests it wrote), double-run
determinism, 8 hashed before/after captures — and documented its cut
lines honestly (per-vertex far dither awaits A's shader pass). Razor
biome lines are gone. Also this phase: weather `season` play command +
the seasons-showcase reel (one forest, four seasons, storms).

## Phase 11 — blending endgame (2026-07-10, small hours)

Austin field-tested continuously and the program closed out its spine in
one night: A shared micro-texture (+ the beach rule the V-1 reshoot
exposed), F one water ramp (the hand-synced copies had already drifted),
B per-pixel shorelines (V-5's angular lake polygons became curved shores
with beach rims; lesson: keep the signed shore field tight around zero —
a -1 km sentinel skews interpolated crossings into notches), E impostor
tuning per field feedback, D's v1 already in. Field finds fixed same-
session: the sun kept shining below the horizon (backwards night trees),
a hole-cut wflag interpolation leak (floating shore-ramp slab), river
shoreline dead-band pits + the river bay/apron family (census total
findings 140.9k -> 40.8k — rivers were hiding a wall family as big as
the lakes'). Sol shipped V-7 (one lake-shore rule, Voronoi-bisector edge
distance, caught the coastal-beach repaint trap); Opus shipped the photo
map relayering (bases + rivers/lakes/clouds-now overlays, pan/zoom with
region re-synthesis). Remaining blending: per-pixel category dither,
E far fade, V-8 deep disc, C knob + palette with Andrew.

## Phase 12 — the night shift (2026-07-10, pre-dawn)

Austin signed off ("anything and everything, your call") and the night
went to work he wouldn't need to witness. The river staircase's LAST
source fell first: the painted vertex wetness — widened so sub-vertex
threads survive at distance — was smearing past the per-pixel field's
crossing on wide rivers; tile_wet now hands bank ownership to the field
once the channel out-resolves the vertex grid (hw/spacing 0.9-2.0).
Then Austin's own late idea became the program's instrument: sync-diff
(voxels on|off toggle + scripts/sync_diff.py) renders 14 poses with and
without the near-field and diffs the pairs — it independently ranked
the lake octave offset worst on the planet (mean 50.6), proved itself
on an open-sea zero, and put numbers on two shading biases nobody had
articulated (polar ice +12, steep slopes -8, ledgered V-9). Per-pixel
normal detail landed on its watch: mesh slopes now shade like stepped
columns (slope-scaled hash normals, water excluded, 2 km fade) and the
river cut banks fused with the patch grain. Frame times became data
(title-bar fps + frame_ms in sidecars) after Austin's "really you can't
gauge that?". Sol ran the V-6 morph mission in parallel. Lesson worth
keeping: the swarm-refinement idea Austin floated at 5 a.m. was half
right — scouting poses with the meter is cheap and good; CLOSING THE
LOOP on the meter is Goodhart's trap, so the meter measures and people
judge.
