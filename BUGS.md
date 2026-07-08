# Bug ledger

Living list of known bugs and irregularities, so no finding gets lost between
sessions or operators (humans, Claude, codex, Opus). One entry per distinct
root cause where known; screenshots referenced by their interchange filename
(pose is encoded in the name). Add new findings at the top of the OPEN
section; move fixed items to FIXED with the commit hash. Repro coordinates
are `teleport LAT LON [ALT_KM]` viewer args at `--exagg 1` unless noted.

## OPEN

### S-2 Banded lighting: cube lee faces collapse to near-black (land + trees)
Reported 2026-07-08 PM (shots at 8.097 -35.673 and the night shot at
4.990 -29.401): terrace risers and tree/canopy side faces band light/dark
harshly, and Mat::Shrub [0.10,0.16,0.06] is so dark an isolated shrub block
reads as a hole in the ground. Root cause: the fragment shader lights
`base * (0.10 + max(dot(n,sun),0))` — a flat 10% ambient, so a face away
from the sun renders at ~1/11th of a sunlit face. Fix direction: a
day-scaled sky-hemisphere fill (~(0.5+0.5*dot(n,up)) * k * day) so vertical
faces get sky light regardless of sun azimuth (the same cure codex applied
to water side faces), plus a brighter dry-brush Mat::Shrub. Night/moon path
must stay untouched; caves/torches multiply after and must keep contrast.

### W-6 Caves near rivers/lakes should be water-filled (polish)
Cave tubes carve under dry land only by intent, but tubes that pass just
below a river/lake water table render dry with the water surface above them
— walk in and the physics/visuals disagree with the hydrology. Flood cave
cells whose ceiling sits below the local water level (needs care with the
walkable-ice and cave-darkness paths). Noted by Austin 2026-07-08.

### W-5 Knife-ridge mountain lakes flood their outer flanks (sim-resolution overhang)
Census-after residual is dominated by TWO high-mountain lakes: level 3282 m
at `-12.1 107.3` (hundreds of grouped sites, walls to 1.6 km) and 3810 m at
`50.91 -28.06`. Their 31 km lake cells overhang knife-edge rims, so ANY
local flood test (Voronoi/rim territory, level margins, basin-floor
comparison — all census-measured 2026-07-08) floods some of the outer
flank, standing the lake surface far above the mountainside, further
complicated by an outlet river carving the same flank. Fix belongs in
bake_rivers.py with whole-lake context: detect cells whose own raster
elevation sits far below the lake level on part of their footprint (steep
rim), and shrink their exported radius / flag them Voronoi-only, possibly
splitting the flood footprint. Everything else in the wall family is fixed
(F-4); this is the last big WALL contributor.

### V-2 Barely-emergent lake shoals read as holes in the water
At grazing angles a flat noise shoal standing <1 m above a lake surface
(e.g. `0.835 67.940`, 0.7 m above the 69.9 m lake) reads as a sunken lens
with a hard dark rim rather than a sandbar/island. It IS land above water
(probed; no water bug) — needs shore/wet-sand material + softer rim
shading. Found while verifying W-1.

### W-3 Voxel quantization staircases on sloping river surfaces
Any sloping river surface renders as 1 m water terraces with exposed side
faces (all river shots). Roadmap already names the endgame ("rapids/
waterfalls where river levels step"). Polish; distinct from W-2 (which is a
spurious LEVEL discontinuity, not quantization).

### W-4 Water staircase side faces alternate bright/near-black
Water wall side quads take `n_side` + sun with 0.8 vary — zigzag walls
alternate harshly lit/dark (very visible in shot_lat4.990 2:07 AM). Consider
a more upward-biased or dedicated water-wall shading.

### V-1 Far-mesh color does not match the voxel landscape
shot_lat0.569_lon68.915_alt0.263km_yaw-149_pitch-25.png: the mesh beyond the
voxel patch renders visibly darker/flatter green than the blocky near field.
Long-term probably texturing/material unification (roadmap may absorb this);
recorded so it isn't lost. Polish, not a correctness bug.


## FIXED

### F-7 (was C-1..C-3) Camera/control requests
Fixed 2026-07-08 (feat/camera-controls bf8bb70, Opus 4.8): above the voxel
range WASD cruising holds constant planet-center radius (rides over peaks,
settles back — asserted <1 m drift over a 16 s mountain flight in
camera-controls.play, now in the verify.sh gate); fly speed far out scales
with radius, capped at 4 planet radii (~0.57x the day's rotation rate);
scroll auto-tilt is behind --auto-tilt, default OFF. Constants are
single-line tunables in player.rs if the feel needs adjusting.

### F-4 (was W-1/W-2) The water-wall family: discontinuous water surfaces
Fixed 2026-07-08 (fix/water-fill-levels, merge fb4d853). Census baseline:
351,711 sites / 1.41M WALL / 288k JUMP / 130 SEAJUMP; four root causes
(bed-anchored river levels incl. ocean bathymetry at mouths and pit-routes,
raw 3r lake flood disc, winner-take-all segment levels, ponds in
floodplains) — full story in the merge commit and ITERATION_LOG Phase 8e.
Post-fix census totals recorded there. Regression: water-walls.play +
lake-regressions.play (which caught the first too-tight flood boundary).
Requires rebake (bake_rivers.py).

### F-5 (was S-1) Concentric terrace-ring shading bands on gentle slopes
Fixed 2026-07-08 (fix/terrace-ring-shading 59608a9, Opus 4.8): top normals
difference the continuous h_km instead of quantized block heights; edited/
carved columns keep the quantized fallback. Verified baseline-vs-after at 4
sites (shade-rings.play); fall-line stripe fix (F-2) preserved.

### F-6 (was T-1) Survey gate had ZERO liquid-lake probes
Fixed 2026-07-08 (fix/survey-lake-coverage a9aeddd, codex GPT-5.5): class
was `interior(lake & tmean>6)` on a planet whose lakes are cold (median
1.7 C) and tiny (median 3 cells) — 0 cells. Now `lake & tmean > -4`
(viewer's freeze threshold), cell-center sampled: 8 probes, all green.
Plus: screenshot JSON sidecars (pose + effective sun) and repro_shot.py
(4bb3ced) make every human screenshot reproducible.

### F-1 Lake fills its Voronoi footprint regardless of fine terrain (lake 414)
Giant wall of ocean-looking water above dry land at `24.5 25.0`. Fixed in
586a828 (flood below-level terrain in the influence disc; damp bed noise by
submergence depth). Regression: scripts/lake-regressions.play. NOTE: the fix
removed the rim boundary and thereby caused W-1 — the coastal escape.

### F-2 Fall-line shading stripes on smooth hillsides
Vertical light/dark bands down grassy slopes (shot_lat-4.498_lon-12.694,
12:26 AM). Fixed in 69a78fa (slope-lit top normals). Verified at exact pose
(runs/stripes). Residual: S-1 terrace rings.

### F-3 Noise ponds terrain-following surface
Fixed in e2425d2 (flat pool level from coarse elevation).
