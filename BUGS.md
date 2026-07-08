# Bug ledger

Living list of known bugs and irregularities, so no finding gets lost between
sessions or operators (humans, Claude, codex, Opus). One entry per distinct
root cause where known; screenshots referenced by their interchange filename
(pose is encoded in the name). Add new findings at the top of the OPEN
section; move fixed items to FIXED with the commit hash. Repro coordinates
are `teleport LAT LON [ALT_KM]` viewer args at `--exagg 1` unless noted.

## OPEN

### W-1 Water-wall family: flood-disc edges + bed-anchored river levels
STATUS: fixes implemented 2026-07-08, verification in progress.
The "pie slice sunken into the ocean" (shot_lat0.813_lon67.967) is NOT the
sea: probes + rivers.bin inspection show a 26-cell lake at level +69.9 m
(nearest ocean cell 119 km away). The sunken capsule is the lake's OUTLET
RIVER corridor: bake_rivers re-anchored river levels to cell BED elevations
even inside lake basins (outlet leaves 20-30 m below the lake surface), and
586a828's 3×radius influence-disc flood stops at an arbitrary circle, so the
below-level corridor past that circle rendered dry/sunken with a stepped
water cliff around it. The planet-wide census (census-baseline.md: 351,711
sites, 1.4M WALL / 288k JUMP / 130 SEAJUMP) shows the full family, worst
case a 3.7 km wall (lake@4303m over terrain@587m, -9.33 105.50).
Root causes and fixes (all landed together):
1. rivers::lake_at floods a raw 3r disc -> restored rim-ring boundary:
   flood covers lake Voronoi territory + a bounded shore band into rim
   territory (noise-dip shores), never past the dam. (terrain.rs decides.)
2. bake_rivers levels were BED-anchored via min-relaxation -> now export the
   HYDRAULIC FILL SURFACE (level = max(elev, receiver level), built up from
   terminals; ocean cells pinned to 0; in-lake nodes pinned to the lake
   level). Kills the deep-negative mouth levels (rivers walled in by the
   sea: shot_lat-0.914_lon-67.773, the 2:13 AM striped-column shot) AND the
   bottomless slot gorges where routes crossed dry pits (census showed
   2-4 km deep carves at e.g. -41.87 -120.47 and -9.33 105.50).
3. river_near returned winner-take-all segment levels -> levels now blend
   across in-influence segments (see W-2).

### W-2 River water cliff running ALONG the channel at a bend
Temperate valley bend `4.990 -29.403 0.3`: the river surface splits into two
levels meeting along a ~hundreds-of-meters diagonal staircase wall mid-water
(shot_lat4.990_lon-29.403_alt0.047km_yaw37_pitch-29.png and the 12:22 AM shot
at the same site; unchanged by the 02:00 fixes — different code path). The
wall runs roughly parallel to the channel, so this is NOT simple downstream
level drop: it is the nearest-segment Voronoi bisector between two competing
reaches (bend arms / confluence) with different interpolated levels. Fix
direction: blend (or min) levels across all segments within influence instead
of winner-take-all nearest. Also visible: a dry notch in the bank where the
perch guard dries part of the channel edge.

### W-3 Voxel quantization staircases on sloping river surfaces
Any sloping river surface renders as 1 m water terraces with exposed side
faces (all river shots). Roadmap already names the endgame ("rapids/
waterfalls where river levels step"). Polish; distinct from W-2 (which is a
spurious LEVEL discontinuity, not quantization).

### W-4 Water staircase side faces alternate bright/near-black
Water wall side quads take `n_side` + sun with 0.8 vary — zigzag walls
alternate harshly lit/dark (very visible in shot_lat4.990 2:07 AM). Consider
a more upward-biased or dedicated water-wall shading.

### S-1 Concentric terrace-ring shading bands on gentle slopes
shot_lat4.992_lon-29.403_alt0.018km_yaw-10_pitch-36.png: dark rings at every
contour terrace on a gentle hill. Mechanism: commit 69a78fa derives top-face
normals from central differences of QUANTIZED block heights — on a gentle
slope the one-column ring at each terrace edge gets a ~27 deg tilted normal
while flat terrace tops stay radial-bright. The original fall-line stripes
(shot_lat-4.498_lon-12.694, 12:26 AM) ARE fixed (verified at exact pose:
interchange/runs/stripes/hillside.png); this is the residual/transformed
artifact. Fix direction: derive top normals from the CONTINUOUS terrain
height (per-column h_km, the same surface the mesh shades) so voxel tops
shade like the mesh; keep quantized behavior on edited columns.

### V-1 Far-mesh color does not match the voxel landscape
shot_lat0.569_lon68.915_alt0.263km_yaw-149_pitch-25.png: the mesh beyond the
voxel patch renders visibly darker/flatter green than the blocky near field.
Long-term probably texturing/material unification (roadmap may absorb this);
recorded so it isn't lost. Polish, not a correctness bug.

### T-1 Survey gate has ZERO liquid-lake probes
`where.py` / `gen_survey.py` class `liquid-lake` yields 0 cells on seed42_r8
(mask `interior(lake & tmean>6)` too strict — likely `interior()` on 1-2 cell
lakes). The whole liquid-lake bug family was therefore invisible to
auto-survey. Fix the mask and add water-level-sanity asserts.

### C-1..C-3 Camera/control requests (viewer/interchange/requested-changes-and-bugs.txt)
1. Mesh-only altitude range: camera locks to terrain height and bobs while
   navigating — should lock to elevation (constant radius).
2. Far from planet, velocity should scale ~ with radius (capped at some
   distance) so the planet's rotation is visible while translating.
3. Camera auto-pan while descending/ascending should be a flag, default OFF.

## FIXED

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
