# Bug ledger

Living list of known bugs and irregularities, so no finding gets lost between
sessions or operators (humans, Claude, codex, Opus). One entry per distinct
root cause where known; screenshots referenced by their interchange filename
(pose is encoded in the name). Add new findings at the top of the OPEN
section; move fixed items to FIXED with the commit hash. Repro coordinates
are `teleport LAT LON [ALT_KM]` viewer args at `--exagg 1` unless noted.

## OPEN

### V-7 Lake-shore sand band disagrees between renderers (territory-wide)
shade_ground paints lake-shore sand from the VERTEX rule (lake_level
raster band — kilometers wide across a lake's territory, e.g. the lagoon
country at 47.80 14.42 renders the whole mesh tan) while voxels sand only
columns near the waterline (ColCtx.lake_shore). Pre-existing double
implementation (the ledger already records this overlay landing twice);
tonight's unified greener grass raised the contrast so the patch reads as
a green disk on tan. Fix direction: extract ONE shared lake-shore rule
(climate_surface-style) consumed by both — a clean follow-up mission in
the D/B family.

### W-7 Residual liquid walls at merged mega-lakes (upstream of W-5b)
After the shore-apron + bounded-flood fix (072a512) the liquid wall census
fell 114,421 findings/683 lakes -> 11,659/556 (median 21.2 m -> 5.9 m, max
267.5 -> 76.3). The remainder is overwhelmingly ONE family: merged
depression chains exported as mega-lakes (cells r 14-18 km), whose peeled
conduit rims march 30-40 km from the basin and whose per-cell radius flips
make any radius-scaled flood/apron rule jitter across Voronoi seams (probe
pair at 22.277 106.010: apron_past 0 vs 2.3 km across 25 m; worst live wall
76 m at 33.539 23.942). Same upstream disease as W-5b (frozen variant):
don't merge depression chains at export / shrink-or-flag steep-rim cells.
Fixing it in planetgen collapses both entries; code-side epicycles were
measured into diminishing returns (three rounds: 114k -> 26k -> 13.7k ->
11.7k). Gate: census --caps (liquid lake WALL sites as JSON) must trend to
zero. Known cosmetic caveat of the apron itself: at cell-radius flips the
bank floor can step tens of metres — DIRT steps (census-invisible, natural
looking), traded for standing water cliffs; revisit with Andrew alongside
the planetgen fix.

### V-6 A fully morphed LOD child does not reproduce its parent mesh (pop)
Sol review finding 5 (2026-07-09, confirmed by code read + Sol's mesh probe).
terrain.rs build_tile: odd child vertices morph toward a BILINEAR blend of 4
parent heights, but the parent renders two triangles split on a fixed
diagonal — at odd/odd vertices those are different surfaces (the twist term
(h_a+h_c-h_b-h_d)/4). The morph is also radial-only (cannot reproduce the
parent chord's tangential position) and wet_parent re-samples wetness rather
than interpolating the parent triangle attribute. Measured residual at swap:
62.4 m at the highest-peak level-9 tile (face 4, ix 339, iy 308), 14.7 m at a
temperate valley — despite renderer.rs documenting "identical geometry".
Cheap partial fix: match the parent's actual triangle diagonal in parent_h
(kills the twist term, likely dominant). Exact fix needs a 3-D parent delta +
triangle-interpolated attributes — belongs to the TRANSITIONS.md program
(same "one truth, two renderers" family as V-1/V-5). Design-gated: pop
visibility is a taste/priority call for Andrew.

### T-1 The lip census cannot see creek films (verification blind spot)
Sol review finding 13. census --lips finds wet/dry transitions from
terrain::sample points ~60 m apart, then column-tests them; the F-15 creek
film exists only after col_ctx's sub-cell samples, so a sub-1.5 m creek
between two dry transect points is never column-tested. "Census unchanged"
therefore does NOT verify the film introduced no lips or clamp/cave
interactions. Fix direction: a film-specific census walking the canonical
column lattice around low-flow segments (enumerate columns the sub-cell
predicate selects; compare rendered water top to all 8 neighbors).

### S-3 Frozen shore cliffs (ice sheets wall above dry ground)
census --lips: most of its 55k sites are FROZEN lakes/rivers whose walkable
ice sheet ends in a multi-block cliff above dry ground (biggest are the W-5
family at high frozen mountain lakes). The liquid clamp deliberately skips
ice (physics stands on it). Whether an ice-shelf edge cliff is even wrong
is a taste call — an ice shelf HAS an edge; the extreme cases are W-5.

### W-5b Frozen summit-lake ice cliffs (residual of W-5, remote + frozen)
After the W-5 bake fixes (8047b27) the wall family's residual is ~600 m ice
cliffs at frozen lakes on the 7-8 km summits (e.g. `-5.86 106.71`,
`40.83 -91.98`) — same merged-depression pathology at the planet's most
remote spots, all below -40 C so they render as walkable ice. The honest
upstream fix is in PLANETGEN: don't merge depression chains into single
lake ids (then delete the bake-side peel). Backlog; census-w5d.md is the
inventory.

### V-5 Lake footprints alias into angular polygons at coarse LOD (promoted)
Round-6 hunt: from ~1 km up, big-lake shores and islands render as broad
straight-edged polygons with orphan blue cells inland (13.357 -4.861; frame
in interchange/runs/round6-findings-repro/). The Phase-7 roadmap item
("needs a soft distance-to-shore signal from the lake index") is now the
most visible artifact from altitude. Walking-height is fine.

### V-1 Far-mesh color does not match the voxel landscape (MOSTLY RETIRED)
MAJOR IMPROVEMENT 76ed4aa (TRANSITIONS A+F): the mesh now carries the
blocks' block-scale brightness fabric per fragment (fading by ~2 km), the
liquid water ramp is one shared function (the copies had drifted 0.28 vs
0.30 blue), and the beach is one shared fraction (blocks dither on it,
mesh mixes by it — the v1_color pose's sand-disk-on-grass is gone).
Remaining residuals: (1) at partial beach/ecotone fractions blocks
speckle while the mesh blends — per-PIXEL categorical dither riding the
A shader cells is the follow-up; (2) V-7 lake-shore band (own entry);
(3) final texture pass still pending (Andrew). Original entry follows.
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

### F-22 Pond water walls (Austin's photographed 18 m wall) — d97527e
The noise-pond mask ended mid-relief exactly like lake territories did:
pond water in walls over dips (shot_lat-0.798_lon-67.941, worst 300+ m in
census). Fixed by interior benches (underwater ground under the pool, never
under existing water), pn-graded shore aprons (valley-gate included — the
likely anatomy of Austin's similar-looking RIVER stretch, unconfirmed until
he finds it again), and three spawn gates: calm relief (env0 < 0.13),
coarse ground < 2% grade, never inside lake flood territory. Census sites
52.9k -> 20.1k; every class below its pre-fix baseline. Five iterations
were census-falsified before this landed (unconditional interior flooding
measured 79.5k pond walls + 141k jumps — recorded in the commit so nobody
retries it). WORLD-VISIBLE for Andrew: ponds vanish from craggy/high/
sloped country and lake basins; pond interiors are shallow pools, not pits.

### F-16 The liquid water-wall family (Sol findings 1+3) — 072a512
Shore aprons + 2.6 r flood bound + carve-wide river ownership in the shared
sample(); census -90% findings, worst 143 m wall at 4.377 39.078 now a
1-block shoreline (interchange sol1_wall_*{before,after}*.png). Residual is
W-7 (open). The render-vs-physics divergence (finding 3) shrinks with the
data; the census --lips survey stays its gate (LIP 0 planet-wide).

### F-17 Ocean mask disagreed across cube-face seams (Sol finding 2) — bb16cba
Per-face edge-clamped blur made the derived ocean fraction differ at the
same world direction (half-navy/half-sand vertical split at -14.457 -45.0).
Ghost-ring border re-blur + canonical shared texels; property test marches
all 12 edges + 8 corners (12,360 pairs, fail-before/pass-after). By Opus.

### F-18 Frozen ice solid only to some world queries (Sol finding 4) — 0ec7b8d
surface_height_km/ceiling/raycast/torches now see the ice sheet (aim, build,
collide, torch height); invariant-survey locks surface==support on sea ice
and a finite ceiling from under the sheet.

### F-19 NaN input + corrupt-save robustness (Sol findings 6+7) — d079d31
Finite-only numeric parsing (CLI + photo map), total_cmp torch sort,
record-validated loaders (face<6, on-lattice, bounded delta), atomic saves.

### F-20 Determinism/input/UI sweep (Sol findings 8,10-16) — c4f171b
Explicit render time (byte-identical play runs, flicker phase by torch
identity), Arc world snapshots per chunk build, key auto-repeat gating,
strict assert semantics, preview staleness, transactional trash moves,
caveprobe seam canonicalization. By codex GPT-5.5.

### F-21 Photo restore reproduces the photo (Sol finding 9) — 31acf19
Sidecar ground_km + mode + solar phase rescaling, seed-gated exact restore.

### F-13 (was W-6) Flooded caves — RESOLVED, see the entry moved below
### W-6 Caves near rivers/lakes should be water-filled (polish)
Cave tubes carve under dry land only by intent, but tubes that pass just
below a river/lake water table render dry with the water surface above them
— walk in and the physics/visuals disagree with the hydrology. Flood cave
cells whose ceiling sits below the local water level (needs care with the
walkable-ice and cave-darkness paths). Noted by Austin 2026-07-08.
MITIGATED 0707a2f: the cave band kept clear of river corridors and
near-waterline lake shores entirely (a mouth breached a river bank at
3.726 63.065 — dry pit below the water table, photographed).
RESOLVED (feat/flooded-caves): true flooded caves. The 0707a2f suppression
is lifted; instead ColCtx carries a per-column `cave_water` table (river
graph level within a bank band, lake spill level within its shore band, or
sea level on the coast, never perched above the column's own surface).
Carved cave cells at/below it render as water (free top surface under an air
pocket / open pit, side faces only into dry cave passages — never a wall over
lower dry ground) and `water_surface_km` reports the sub-surface pool so a
player swims (verify: scripts/flooded-caves.play — underwater, has_water, no
fall-through, swim-up-and-out, and digging a shaft down floods it; frames in
scripts/flooded-caves-visual.play). Cave water is always LIQUID (underground,
never the walkable-ice path); the single-surface model still can't show an
air pocket that a player must reach through a fully-submerged passage — such
a pocket reports as submerged. The old dry pit at 3.726 63.065 now floods.

### F-14 (was W-3) Rapids foam where liquid rivers step
Fixed 2026-07-09 (fix/river-micro 6dbf96f, codex): liquid water-to-water
steps tint their exposed face + a lip of the upper surface toward pale
foam; frozen cascades and shoreline faces exempt. Verified before/after at
gentle, mountain, and icy sites.
### F-15 (was V-4) Sub-voxel creeks keep a 1-block film
Fixed 2026-07-09 (6dbf96f): narrow channel cores whose water rounded away
force water = ground+1 when the sample is genuinely wet (liquid, non-sea,
non-lake, unedited). Dry washes stay dry (guard verified); wading is
ankle-deep; census --lips unchanged (no new shoreline lips).

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
