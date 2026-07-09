# Bug ledger

Living list of known bugs and irregularities, so no finding gets lost between
sessions or operators (humans, Claude, codex, Opus). One entry per distinct
root cause where known; screenshots referenced by their interchange filename
(pose is encoded in the name). Add new findings at the top of the OPEN
section; move fixed items to FIXED with the commit hash. Repro coordinates
are `teleport LAT LON [ALT_KM]` viewer args at `--exagg 1` unless noted.

## OPEN

### V-4 Sub-voxel creeks: mesh paints them, voxels can't show them
A creek narrower/shallower than ~a block renders as painted blue on the far
mesh but has no voxel water blocks up close — reads as the river drying at
the patch boundary. Related: a prominent DRY noise gully next to a painted
river reads as the same river gone dry (legit but confusing — the reported
9.75 30.23 case probed as this). Options: minimum 1-block water film for
in-channel columns, or fade the mesh paint at sub-voxel width. Census
--lips exists now (2026-07-08) for measuring shoreline classes; its liquid
tail (bank-cliff-with-water-film sites) needs triage.

### S-3 Frozen shore cliffs (ice sheets wall above dry ground)
census --lips: most of its 55k sites are FROZEN lakes/rivers whose walkable
ice sheet ends in a multi-block cliff above dry ground (biggest are the W-5
family at high frozen mountain lakes). The liquid clamp deliberately skips
ice (physics stands on it). Whether an ice-shelf edge cliff is even wrong
is a taste call — an ice shelf HAS an edge; the extreme cases are W-5.

### W-6 Caves near rivers/lakes should be water-filled (polish)
Cave tubes carve under dry land only by intent, but tubes that pass just
below a river/lake water table render dry with the water surface above them
— walk in and the physics/visuals disagree with the hydrology. Flood cave
cells whose ceiling sits below the local water level (needs care with the
walkable-ice and cave-darkness paths). Noted by Austin 2026-07-08.
MITIGATED 0707a2f: the cave band now keeps clear of river corridors and
near-waterline lake shores entirely (a mouth breached a river bank at
3.726 63.065 — dry pit below the water table, photographed). True flooded
caves remain the open feature; this entry stays open for that.

### W-5b Frozen summit-lake ice cliffs (residual of W-5, remote + frozen)
After the W-5 bake fixes (8047b27) the wall family's residual is ~600 m ice
cliffs at frozen lakes on the 7-8 km summits (e.g. `-5.86 106.71`,
`40.83 -91.98`) — same merged-depression pathology at the planet's most
remote spots, all below -40 C so they render as walkable ice. The honest
upstream fix is in PLANETGEN: don't merge depression chains into single
lake ids (then delete the bake-side peel). Backlog; census-w5d.md is the
inventory.

### W-3 Voxel quantization staircases on sloping river surfaces
Any sloping river surface renders as 1 m water terraces with exposed side
faces (all river shots). Roadmap already names the endgame ("rapids/
waterfalls where river levels step"). Polish; distinct from W-2 (which is a
spurious LEVEL discontinuity, not quantization).

### V-5 Lake footprints alias into angular polygons at coarse LOD (promoted)
Round-6 hunt: from ~1 km up, big-lake shores and islands render as broad
straight-edged polygons with orphan blue cells inland (13.357 -4.861; frame
in interchange/runs/round6-findings-repro/). The Phase-7 roadmap item
("needs a soft distance-to-shore signal from the lake index") is now the
most visible artifact from altitude. Walking-height is fine.

### V-1 Far-mesh color does not match the voxel landscape
shot_lat0.569_lon68.915_alt0.263km_yaw-149_pitch-25.png: the mesh beyond the
voxel patch renders visibly darker/flatter green than the blocky near field.
Long-term probably texturing/material unification (roadmap may absorb this);
recorded so it isn't lost. Polish, not a correctness bug.
ESCALATED by the round-6 hunt (2026-07-08 night): over flat terrain from
low flight the voxel patch reads as a distinct CIRCULAR DISK of
high-frequency texture against the flat far mesh (7.042 33.477 alt 0.8;
frame in interchange/runs/round6-findings-repro/patch_disk_mouth.png) —
a patch-footprint dome, not merely a color shift. Centerpiece of the
texturing conversation with Andrew.


## FIXED

### F-12 (was W-5) Degenerate sim lakes flooded mountain flanks
Fixed 2026-07-08 night (8047b27): planetgen merges depression chains into
one lake id (lake 873: beds 588..3279 m under a 3282 m spill). Bake now
peels conduit cells (>300 m below spill; healthy lakes are shallow dishes,
p99 depth 376 m), caps every lake's level so its territory-edge cliff
cannot exceed ~15 m against the BLENDED terrain (223 unrenderable lakes
export dry), and rims carry their elevation so shore-band flood-through
only crosses true dams. Census: max wall 1653 -> 634 m, >100 m sites
-43%, total magnitude -27%. Residual: W-5b (frozen summit ice cliffs).
The regression-gate catch that re-locked lake-414/pond suites exposed
that 414's sim level had ALWAYS stood as 131 m walls at its unphotographed
perimeter — the suites now lock corrected levels.

### F-11 Riser-bake smears: step-dense terrain read as dark bands
Fixed 2026-07-08 evening (0707a2f). The third and final layer of "banding":
riser faces baked at 0.72 brightness (a relief cue from before slope-lit
tops existed) compounded wherever steps are dense — terraced washes and
meander banks read as dark smears tracing the channel. Bake lifted to 0.90;
slope shading + sky fill carry the modelling. Layers, for the record:
F-2 fall-line stripes (top normals), F-5 terrace rings (continuous
normals), F-8 lee-face collapse (sky fill), F-11 riser bake (this). What
remains is per-corner AO flecks and taste — texturing territory (V-1).

### F-8 (was S-2) Banded lighting: cube lee faces collapsed to near-black
Fixed 2026-07-08 (fix/sky-fill-lighting 94506c6, codex GPT-5.5): the shader
lit faces with a flat 10% ambient (lee faces at ~1/11th of sunlit). Now a
day-scaled sky-hemisphere fill — ambient = 0.10 + 0.40*day*(0.5+0.5*n.up),
sun term compensated to keep sunlit tops exactly at the old brightness.
Night pixel-identical (diff max_abs 0); noon +0.73% luma. Mat::Shrub
brightened from near-black to dry olive [0.22,0.25,0.10].

### F-9 (was W-4) Water step side faces alternated bright/near-black
Fixed 2026-07-08 (fix/water-visual-polish 8891546, codex GPT-5.5): side
faces take an up-biased normal (outward*0.18+up) and 0.93 of the surface
color — steps read as water edges under any sun.

### F-10 (was V-2) Barely-emergent lake shoals read as holes in the water
Fixed 2026-07-08 (fix/water-visual-polish 8891546, codex GPT-5.5): dry
ground within 1.5 m above the local lake level renders as sand on both the
voxel and far-mesh paths (temperature-gated, tree-free) — shoals read as
sandbars, e.g. `0.835 67.940`.

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
