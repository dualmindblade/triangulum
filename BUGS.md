# Bug ledger

Living list of known bugs and irregularities, so no finding gets lost between
sessions or operators (humans, Claude, codex, Opus). One entry per distinct
root cause where known; screenshots referenced by their interchange filename
(pose is encoded in the name). Add new findings at the top of the OPEN
section; move fixed items to FIXED with the commit hash. Repro coordinates
are `teleport LAT LON [ALT_KM]` viewer args at `--exagg 1` unless noted.

## OPEN

### L-1 COMPLETE — all three observations fixed (a/b: ced643a, c: 74bfce7)
(a)+(b) fixed by Sol's border pass, merged ced643a: range comparator
octaves retire at screen Nyquist, ownership goes hard at the filtered
prefix (~1 px AA, BIOME_BORDER_AA_BAND_PX dial), medium-range
categorical deviation converges toward range_mean (dial composes with
the orbital one). Far borders read as the near view's ragged fractal;
far tint predicts arrival. World unchanged (shader-only); 2 orbital
reel poses re-blessed (means 0.33/0.41). After-captures at Austin's
titled poses: interchange/borders-after-merge/ (befores = his archived
photos). NOTE: worktree evidence was deleted before archiving —
FINDINGS gist lives in the merge commit message; lesson: archive
FINDINGS + sheets BEFORE git worktree remove.
### L-1 historical entry (superseded):
2026-07-17: (c) flat mid-altitude color FIXED by the ground patchiness
field (patch_multiplier in shader.wgsl, sibling of strata_multiplier):
three world-anchored octaves ~1.4 km/350 m/90 m modulate ground tone,
warm-bright/cool-dark, per-octave footprint retirement (no orbital
shimmer/cost), water excluded, snow at 0.35 strength. Austin approved
from A/B at his titled poses (interchange/runs/l1c-patch/). Knobs:
PATCH_STRENGTH 0.11, PATCH_FREQ_BASE, warm/cool skew — for Andrew.
ROOT CAUSE for (a)+(b), one mechanism seen from two directions: the
categorical ecotone comparator is already fractal to column scale, and
the fragment shader ANTIALIASES it to the interpolated class mean as
octaves go sub-pixel — far view = smooth wavy blobs + averaged tint
contrast, near view = crisp dither + dissolved tint contrast. Fix
direction (deliberately after Track B merges — same fs_main region as
the waterfall shader work): keep the class decision crisp at the
coarsest still-visible octave instead of melting to the mean, and
converge far tint contrast toward the near field's effective palette
so borders neither pop at range nor vanish on approach.
Original observations follow:
### L-1 original entry (2026-07-16): Austin+
Andrew, three related observations that should be tuned
against REAL campaign content, not today's sparse world: (a) medium-
high altitude renders wavy smooth borders where the next LOD in is
rougher and better ("smooth"/"not so smooth" photos) - the coarse
levels lack a boundary-detail octave; (b) some biome borders are
visible at range and vanish on zoom ("here we see a border"/"here we
don't"); (c) flat-color landscapes at medium altitude - needs
mid-scale ground texture, which Track C strata banding + geology
directly provide. Schedule: after geology lands, one focused pass.





### CROSS-REVIEW 2026-07-14 (Sol reviewed Claude 14aed27..999ed70; fresh
### Claude context reviewed Sol's MP1/W2.5/W2/moon). Fixed same-day items
### are in FIXED; these remain open:

### R-5 NOTE (MP1, remaining half): the game token travels in the URL
query (/?token=...). Cloudflare's edge may log full request URLs, and
the token is the sole credential. Move to a header or websocket
subprotocol during MP2/MP3 (invite-format change; coordinate both
sides). The other R-5 items (timing-unsafe compare, dead import path)
were fixed 2026-07-14 with R-1..R-4.

### R-6 MINOR (weather): lifecycle arithmetic degrades at absurd times
At t ~ 1e20 s the epoch remainder cancels to the boundary and every
cyclone intensity is exactly 0 (storms silently vanish; still finite
and deterministic). Normal play/travel can't reach it (1e20 s ~ 1e13
game years); the f64::MAX NaN half of this finding was fixed same-day
(zonal_shear_phase op order). If the time-travel UI ever accepts freeform
huge years, clamp there.

### R-7 NOTE (weather perf): structured cyclone shading (arms + comma
front: per-system frame build, atan2, log per sample) costs ~4.6% avg /
~5.8% p95 on the cyclone orbit bench vs 14aed27 (Sol measured; ground
view unaffected). Fold into the perf campaign's shader pass.

### B-8 High-orbit cloud deck reads as uniform fine speckle — RESOLVED W2.5 (uncommitted)
W2.5 resolution (2026-07-13, `sol/w25-heterogeneous`): the deck no
longer derives live shape from the camera-to-planet-mean scalar. A pure
6x64x64 RGBA8 cube raster bakes field cover/precip at deterministic
two-second weather-time buckets; every low/mid/high shell hit samples those
same bytes per pixel, while its existing fabric supplies formation detail.
The teleport map reads the identical raster. Zero-cover live regions now
gate out the old inverse-cover cirrus presence, so orbital clear lanes stay
clear instead of becoming maximum fine speckle. Exact pins still upload a
uniform raster and take the legacy scalar/presence path, preserving reel
comparability. The t=3500 contact gate shows regional belts and local fabric
at 12,000/4,000/500 km, and a north-up map/sky pair visibly shares the two
continents, central strait, and thick/clear lanes. This entry remains under
OPEN only until the mission lands and has the required fixing commit hash.

At ~1000 km with live or pinned cover, the deck renders as global
salt-and-pepper dust rather than cumulus masses; the pre-W2 control
(d90835d) renders NO deck at that altitude at identical pins, so
"clouds at high orbit" is itself new W2/cyclone-era behavior whose
current form is wrong. Repro: scripts/cyclonehunt.play +
scripts/pinhunt.play, compare against a d90835d control build.
Bisect W2 (8449a0d) next; suspects: deck handoff/altitude constants,
per-layer shell-hit geometry at high range, presence gates at
low-cover. NOTE the confounder that burned an hour: mean-diff
metrics cannot judge this - LOOK at frames. The W-MOTION pass-1
fabric terms (warp/shear/morph) were reverted 2026-07-13 after the
same hunt exposed them as a regression zoo at high weather clock
(paisley marbling, octave decorrelation, variance washout); the
pass-1 redo requires visual gates at t in {0, 1800, 3500} minimum.

### B-4c OPEN: forest cold builds are the REAL remaining B-4 class
The B-4a cache was REVERTED (b372b70) after Austin field-caught a
regression over productive forest (see PERF.md item 1 for the full
post-mortem + re-mission requirements). Current truth at HEAD:
(1) dense-forest cold tile builds still wall ~3.2 s at the 2 km
descent step of b4a-forest-descent.play - triggered live by
HORIZONTAL flight bringing new forest regions over the horizon;
(2) pose 80.909 -74.619 keeps a ~220 ms non-impostor scheduling
floor at 2 km (persists with TRI_NO_IMPOSTORS=1); (3) an exit-101
crash Austin hit once during descents remains unreproduced (was
possibly the reverted cache path - watch for recurrence); (4) the
never-block ancestor-draw scheduling decision is banked for Andrew
and would cure every urgent-build hitch class at once.
Left open by the B-4 fix (Sol mission 2026-07-14, findings archived in
interchange/reviews/sol-b4-findings-2026-07-14.md): dense or
climate-boundary level-11 tiles that satisfy neither exact region
proof still enumerate large candidate sets, so the very first cold
teleport can spend ~2.6 s (down from 3.75 s). The structural fix is a
reusable lean candidate stream/cache keyed independently of tile LOD -
the perf campaign's next item. Also unattributed: the mixed cold
upload maximum (~236 ms single upload) needs terrain-vs-voxel
accounting before touching buffer batching.


### B-6 Oblong craters cast circular halos/rays
Side-impact (elongated) craters keep a CIRCULAR proximal halo and
ray field: apply_ray_field passes elongation 1.0 and the halo radial
test ignores the crater frame. Rays/halos should inherit the crater's
elliptical frame (and real oblique impacts throw asymmetric
butterfly ejecta - r3 material, see SOLAR.md roadmap).

### B-7 Crater seams at cube-face boundaries
Andrew's photos "Cube face boundary inside crater" (31.315 44.679)
and "Crater shape discontinuity" (48.042 -37.405): crater fields
change across cube-face seams - the per-face cell enumeration or the
canonical-owner projection disagrees at the boundary. Same family as
the (fixed) B-3 gate-cut cliffs; probe with rimtransect-style scans
across the seam.

### B-2 Moon crater singularity ("black hole") — FIXED by moonscape v2
Verified at merge review: the site renders as ordinary regolith plain,
probe height -0.179 km (a legitimate crater floor), and
scripts/blackhole.play stands as the regression (physical +-2.5 km
bounds at the exact coordinate). Original entry follows.
### B-2 Moon crater singularity ("black hole") at -35.057 51.770
Andrew's photo (titled "Black hole"): terrain spirals into an
unbounded pit - a crater depth/dimple formula diverging at its exact
center (divide-by-near-zero shape). In moon.rs's P2 crater fold,
which the moonscape v2 mission is REPLACING wholesale - so the gate
moves to the v2 merge review: probe this exact coordinate, and the
new lattice system must clamp every depth/radius ratio (add a unit
test: no |height| beyond physical crater depth bounds at 1e6 sampled
directions).

### B-1 Distant impostor treeline reads as a faint grid (banked)
At the outermost impostor ring (level 11 -> 10, where impostor_stride
ends) forests stop at tile boundaries - a subtle grid-aligned treeline
at the altitudes with the pre-existing build-cost issues (Austin
2026-07-12, "quite subtle", banked by agreement). Proper fix rides the
lean-impostor-vertex perf mission: encode the tree lot in beach.y and
distance-fade trees per-fragment so density reaches zero BEFORE the
ring ends (continuous treeline into the far forest tint).

### F-24 Deep-tile stand-ins: double grids, mesh-among-voxels, ground
'clouds', multi-second frames — FIXED 79cee18
Andrew's 2026-07-12 field report (photos at -44.507 -114.451 and
-44.483 -114.444): two incoherently-offset grids on ground voxels,
mesh apparently rendering with the voxel map, pale cloud-like tint on
terrain, frames of 2+ s in the same areas. One root cause: the async
tile pipeline (a6f5976) let DEEP tiles defer, drawing their non-deep
ancestor as a stand-in - 8-octave geometry meters off the 12-octave
patch, in w=1 far-tile mode (the soft water tint = the 'cloud' film).
The no-stand-in fallback also built sequentially per key (the
multi-second bursts). Fix: deep keys always build urgently; leftovers
build as one parallel batch. Reel 24/24 byte-identical; groundhop.play
worst frame 190 ms on 200 m ground hops. Verify in-game near any voxel
patch while flying laterally.

### F-25 Dusting dither banding (both of Andrew's banding reports) —
FIXED c833071, completed c334289
Act two/three (2026-07-12 overnight): the hash3i cell-constant cure
re-read as a 25 m checkerboard whose borders crawled with the camera
- raw planet-centered f32 positions quantize at ~0.24 m. Final form:
camera-anchored exact integer lattice (danchor globals) + two smooth
vnoise octaves. Residual static checkerboard under heavy dust at
ground level = designed D-2 block mottling (bisect: checksect.play).
Regular stripes at 50.759 -86.619 (07-11) and irregular N-S stripes +
ground white-out at 60.704 89.662 (07-12, 'not aligned with voxel
grid' - correct, it wasn't the lattice). The snow-dusting breakup
dither fed hash31 cells at planet magnitude (~82k) where the fract()
hash degenerates into axis shards - the documented failure that
already moved the cloud deck to hash3i. Same cure, same 25 m cells;
both renderers share the path. Repro: bandrepro.play / bandrepro2.play
pin the photos' exact weather times (sidecar t_s -> weather time).
NOTE: the reel pins weather, so dusting appearance is OUTSIDE reel
coverage - the repro scripts are the specimen pair; a dusted pose
belongs in any future weather-visual reel.

### A-4/A-5 FIXED on the second pass (gauntlet-constrained redo, 29188ec)
FIXED: the dam family (water, dig, bench, fill, apron) now suppresses
together and continuously - rampart site LIP/EDGE 215/136 -> 0/0, all
67 local WALL sites including P-1's 28 CLOSED; the rim band is gone
without touching block occupancy or lake shapes. World reel: only the
two bug-zone poses changed, 22/24 byte-identical, VOID lint silent,
physics swim checks pass. DESIGN NOTE for Andrew: the large
dam-dependent pond at 12.199 -44.827 no longer exists (it required a
dam beyond budget) - the zone is savanna with dry traces now; if
dammed ponds are wanted as a FEATURE, that is a new design item, not a
bug. Original reopened entry follows.
### A-4/A-5 Pond ramparts + rim water band (REOPENED after water2 revert)
The first fixes (72448da: 3 m dam cap; analog water tops shared by
render+physics) were REVERTED at 0d70b29: Andrew's survey found a
regression zoo the mission gates could not see - a grid on open water,
black void cracks / invisible water along shores, Difficulty Lake
reading a different shape, dry pond beds with banks and 1-block water
films near the rampart site. The redo MUST gate on the four dossier
zones (poses in scripts/revert-check.play, clean reference frames in
interchange/runs/revert-check/): byte-compare-grade A/B at each zone,
a near-black void-pixel detector on daytime shore captures, colcensus
+ census at both pond sites, and the original two bug poses. Original
diagnoses hold: rampart = pond bench/apron dam along the pn contour;
band = block water tops at the lattice ceiling standing proud of the
mesh plane at the patch rim.

### P-1 Residual pond walls at the rampart site (28 census sites)
After the rampart cap + fill taper (5f6d57e), census --at 12.194 -44.858
still reports 28 WALL sites (worst d=23.6 m, pond@28.2 over dry@4.6,
plus 3 more at 12.181 -44.853). Identical under the clamp and taper fill
forms, so the wall stands somewhere else in the pond machinery (apron
path or a mask-interior bench against unbanked exterior). Probe before
hypothesizing - this family has a graveyard (Phase 8h). Gate: the site's
WALL count to 0 without raising global census totals.

### V-8 The deep-tile disc: full-octave detail ring pops against coarse mesh
From ~2 km looking straight down (0.625 68.960 pitch -79), the deep-tile
ring around the voxel patch reads as a lighter, busier disc: octave-12
sampling resolves river carves and dirt banks that octave-8 neighbors
smooth away. Wetness is already octave-stable (RIVER_REF_OCTAVES), but
CARVE VISUALS and relief shading are not. Pre-existing (deep tiles exist
precisely to match the blocks); A's shared texture fixed the flat-field
half of the old V-1 disc, this detail ring is the tail. Fix directions:
taper the extra octaves across the deep ring instead of stepping, or
gate bank-dirt exposure by an octave-stable reference like the perch
rule. Low urgency: visible only under ~2.5 km looking near-straight down.

### V-7 Lake-shore sand band disagrees between renderers (FIXED 984efc8)
Fixed by GPT-5.6 Sol: terrain::lake_shore_frac is the one lake-beach rule
(1.5 m height band x 100-300 m of the lake/rim Voronoi-bisector edge,
liquid only); mesh mixes by it, blocks dither on it, coastal beach_frac
yields inside lake territory (the repaint trap). Lagoon province is grass
with a true shore rim. Unit-tested; suites green. Original entry follows.
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

### V-6 A fully morphed LOD child does not reproduce its parent mesh (FIXED 1e2a40b)
FIXED by Sol (2026-07-10 overnight, its own review finding): morph targets
now interpolate over the parent's ACTUAL rendered triangle (same fixed
diagonal as the index buffer — the twist term is gone), and morph_wet
interpolates the parent triangle's real paint. Measured: 62.37 m -> 0.127 m
(peak), 14.73 m -> 0.110 m (valley), wet 0.364 -> <1e-6. The exact vec3
target (~1 mm) was evaluated and REJECTED: +8 bytes on every shared vertex
(+11.8% bandwidth) for 0.13 m at swap distance. Sol also removed the
equal-octave zero-delta shortcut (never a valid parent reproduction).
Permanent gate: terrain::tests::full_morph_reproduces_parent_triangle_at_
v6_sites (topology-aware — reads the parent index buffer; old code fails
250x over); examples/morph_probe.rs reproduces the numbers. Original
entry follows.
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

### T-1 The lip census cannot see creek films (CLOSED by colcensus)
CLOSED 2026-07-10 (hunt-prep tooling): examples/colcensus.rs walks the
canonical COLUMN lattice in a disc and classifies block-truth water
against each column's 4-neighborhood — LIP (2+ deep water above dry
ground), EDGE (1-deep standing edge: the analog-clearance dead band ring
AND creek films), CAVEP (surface-breaching cave pools). It sees
everything Sample-level tools cannot (films, cave water — the class that
hid the whole karst saga). Baseline at Difficulty Lake 0.2 km: EDGE
1,341 / CAVEP 3,909 / LIP 0 — the EDGE count is the numeric gate for the
lake dead-band fix. Companions: the play harness `probe LAT LON` command
(sampler + column truth in-run) and `sync_diff.py --at LAT LON ALT`
(ad-hoc pose diff). Original entry follows.
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

### W-8 Weather fidelity cluster (Sol review #2 findings 2,3,4,7,10,13)
FIXED IN WORKTREE, AWAITING CLAUDE REVIEW (Sol #6, 2026-07-10). Six related weather gaps:
(2) recorded shot weather is dead metadata - neither repro_shot.py nor the
photo-map restore replays it (breaks the "photo of a storm is a coordinate"
contract); (3) the bake drops WEATHER.md's promised semiannual harmonic -
monsoon/bimodal regimes collapse to one sinusoid (k=2 would cut global
precip RMS 35.1 -> 24.9 mm/mo); (4) synoptic advection uses
normalize(dir - drift), whose angle asymptotes at 90 deg - fronts freeze
after ~a simulated year; (7) weather_tuning.json is unvalidated -
days_per_year: 0 NaN-poisons every uniform; (10) the photo map's
Clouds-now ignores weather off/pin and never resynthesizes on time alone;
(13) `weather season FRAC` sets epoch_frac, so after any elapsed sim time
the frame season is FRAC + elapsed. Gates: byte-determinism reel, suites,
before/after demos per item.

Resolution: WEA2 adds the promised precipitation/cloud k=2 terms (global
RMS 35.08032 -> 24.84529 mm/month and 0.03739 -> 0.02319 cloud fraction),
advection is an exact great-circle rotation through arbitrary elapsed time,
shot repro/photo teleport restore weather via an independent absolute clock,
Clouds-now mirrors off/pin/live with a 60 s refresh bucket, invalid tuning
falls back loudly as a whole, and `weather season` subtracts elapsed phase.
New unit gates cover WEA1/WEA2 loading, two-year advection, tuning rejection,
season semantics, opt-in photo restore, and map bucketing. Full evidence and
gate transcript: `viewer/interchange/codex/FINDINGS.md`.

### E-2 Forest impostors skip tree_here guards (review #2 finding 9)
The impostor lottery matches tree placement but not full eligibility:
tree_here also rejects edits, 8-neighbor relief, neighboring carves, and
cave mouths (top_solid != ground). Measured: 23 of 915 impostors on the
jungle reference tile stand where no block tree will exist (first:
15.946564 -78.370195, a cave mouth) - a billboard rooted over a pit at
the forest handoff. Fix shape: run survivors through the cheap parts of
tree_here (one col_ctx per survivor after the two-phase cut).

### V-6b A fully morphed child does not morph its shore field (review #2 f12)
V-6 morphs height + wetness to the parent triangle, but Vertex::shore
passes through unmorphed: 8 sign disagreements on the lake_shore test
tile (child -0.064 m vs parent +0.073 m at 13.343049 -4.821302) - the AA
shoreline can flip sides at the LOD swap. Fix shape: parent-triangle
shore grid + one more morph channel, weighed against V-6's own vertex-
size argument (a shore f32 is +4 bytes; evaluate before paying).

### V-10 Flooded-cave surface breaches are voxel-only water (HINT SHIPPED, v2)
V2 (same day, Austin's field survey at the newly-christened DIFFICULTY
LAKE, 13.346 -4.807): (1) false pools — a cross-section "core depth"
proxy flooded tube cores whose carve never reaches the table (diff at
13.349 -4.798); the flood test now probes the SAME field 2.5 m deeper
(the blocks' actual geometry question: does the tube continue below the
table?) — false pools 2,974 px -> 0 at that pose, main channel IoU held
at 0.48, error sign now conservative (rare false-dry, never invented
water). (2) dry pit mouths recolored from a plain darken (olive smears
on grass, photo at 13.336 -4.798) to Mat::Dirt albedo lightly shadowed —
the sync-diff dry poses now BEAT their pre-hint baselines (savanna 7.72
-> 7.35, desert 6.65 -> 6.77): drawn pits in dirt match blocks better
than absent pits. V1 notes follow.
MEASURED LIMIT (review #2 finding 8): the u32 lattice hash is bit-exact,
but kgnoise evaluates in f32 vs the CPU's f64 - max noise delta 0.020434
flips 68 of 25,921 surface tube classifications (0.26%) near the 0.085
threshold; this is the arithmetic root of part of the ~250 px false-pool
floor. Accepted for v1 (WGSL has no f64); revisit only if edge shimmer
is ever visible in motion.
V1 MESH HINT LANDED (2026-07-10, Austin: "karst, frigging awesome" — keep
the breaches, make the mesh show them): the fragment shader now evaluates
the EXACT cave field the columns carve with — kgnoise/khash are bit-exact
u32 twins of noise.rs (every step of its i64 hash is 32-bit masked, so u32
arithmetic reproduces it; premultiplied seeds ride globals.karst; KGRAD
generated from noise_grad.rs). Flooded breaches (shore field carries lake
proximity) join the water pipeline as pools; dry breaches darken into pit
mouths; full strength to the patch rim then fades by ~2 km. Two traps
cost the evening: per-pixel elevation from length(wp) is f32-quantized to
~1 m at planet magnitude and each quantum shifts the tube field ~8 m
(moire scanlines) — fixed by camera-relative expansion (sky.w +
dot(rel,up) + curvature term); and a depth-union "fix" for those stripes
was compensating the same bug — surface-only k=0 matches truth once zm is
precise. Measured: water-mask IoU voxel-vs-mesh render 0 -> 0.48 at the
150 m satellite pose, 0.92 over the 450 m karst field; karst joined the
sync-diff standing table (mean 13.3, lum -0.1); dry-pit hint costs +1-2
mean on cave-bearing dry poses (style mismatch, v2 tune). GENERALIZATION
(the reason Austin greenlit): any surface feature that is a pure function
of (dir, seed) can now get a mesh twin this way — dry breaches already
do; future generated structures should keep their placement pure
functions to qualify. Player edits are not pure functions (would need an
edit-splat overlay). Original entry follows.
Austin's V-key survey find (2026-07-10, sync_lat13.346_lon-4.807 heatmap
"satellites"). Around lake 582 (13.346 -4.807), dozens of pond- and
channel-shaped water bodies render in the voxel patch and vanish in the
mesh render. Diagnosis (a long hunt — see examples/colmap.rs, born of it):
they are NOT ponds and NOT rivers; terrain::sample has NO open-air water
there. They are FLOODED CAVES (the W-6 feature): cave tubes within the
lake's groundwater band (h < lake_level + 10 m) fill to the lake table,
and where a tube grazes the surface the carve opens it into a blue pit —
elongated snakes and blobs that read as ponds/creeks from altitude. Three
consequences: (1) the mesh cannot render them (caves live only in
col_ctx), so they pop in/out at the patch boundary — a transitions seam
in the V-1/V-5 family but structurally unfixable by color sync; (2)
census/sample tooling is blind to them (T-1's cousin: cave water is not
in Sample); (3) water_surface_km reports the pool only at/below its
table (correct for wading), so surveys of the RIM read dry — colmap's
'C' channel is currently the only inventory. Their density near this
lake is a world-design signature in itself (a karst pond field).
Fix directions, all Andrew-gated: (a) suppress near-surface breaches in
the flooded band (world change: keep tubes >N m under open terrain); (b)
accept them as karst and give the mesh a shared breach predicate so far
tiles paint a hint of them (expensive: per-vertex cave noise taps); (c)
accept the pop as-is and let the rim knob C own it. Gate for (a)/(b):
the sync-diff satellites at this pose disappear or match.

### V-9 Quantified shading biases (ICE MOSTLY FIXED ecf0b5d; slopes -8 open)
UPDATE 2026-07-10 evening: the polar-ice bias was mostly a BUG, not taste -
block water/ice quads took cold DUSTING through the ground path while the
mesh wet-mix masked it (exposed when review #2's lapse fix shifted the
reference 0.78 C). Chunk water surfaces now carry wflag=1 and the shader
skips dusting/rain on open water: ice_top mean 19.1 -> 8.8, lum bias
+12.0 -> +3.4. Residual +3.4 and the steep-slope -8 remain the original
shading-share questions below.
### V-9 (original) polar ice +12, steep slopes -8 (sync-diff)
Found by the sync-diff meter (scripts/sync_diff.py, 2026-07-10 overnight).
Two systematic mesh-vs-block brightness disagreements, in 8-bit luminance
over the divergent region: (1) ice_top pose (83.997 40.22): +12.0 bias
across 61% of frame — block ice/snow renders much BRIGHTER than the mesh
ice sheet; (2) steep slopes (peak pose -8.1, both river poses ~-8): block
slope self-shade (F-16 fix) darkens harder than the mesh's continuous
diffuse. Neither is a bug in isolation — they are the two renderers
disagreeing about material brightness, and which side is RIGHT is an
Andrew taste call (does he prefer the brighter block ice or the duller
mesh?). Fix shape: share one slope-shade/ice-albedo rule like
water_surface_color/beach_frac. Gate: the poses' lum bias trends to ~0.

### V-5 Lake footprints alias into angular polygons at coarse LOD (MOSTLY RETIRED)
FIXED IN COLOR (TRANSITIONS B): vertices now carry a signed water-minus-
ground field whose interpolated zero crossing is stepped per fragment
with derivative AA — shorelines render at pixel resolution (organic
curves, no orphan blue cells; flagship 13.357 -4.861 verified at 1 km
and 4 km, v5-check.play). Residual: the lifted water-plane GEOMETRY
still ends at vertex resolution, a faint silhouette wiggle at glancing
angles from low altitude. Field clamps +-5 m (a -1 km sentinel skewed
crossings into notches — first-build lesson). RIVERS joined the field
(wl - h crosses zero at the carved waterline) so their edges stopped
stair-stepping too. Last stair source closed 8eea348: the painted vertex
wetness (widened for sub-vertex threads) smeared past the field's crossing
on WIDE rivers — tile_wet now hands ownership to the field once
hw/spacing crosses 0.9-2.0 (river-zoom-2.png teeth gone, re-shot both
poses). OCTAVE/CLASS RESIDUAL FIXED on sol/v5-shore-octaves: the shore
color channel now completes eligible non-deep vertices to the voxel octave
depth while positions/normals/water-plane ownership remain spacing-capped.
The same change exposed and closed the larger measured error: Sample's lake
predicate requires 0.5 m of water clearance, but the old shore field stepped
raw level-ground at zero and painted those sub-threshold shoals as water.
Sharing that predicate removed the heatmap's whole-lobe class flips:
sync-diff lake_shore mean 50.6 -> 15.39 (divergent 29.0% -> 24.8%), with
sea_calib still 0 and no pose regression. The lifted water-plane GEOMETRY
silhouette residual above remains intentionally separate. Original entry
follows.
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

### B-15 Mesh renders beach as water one level above voxels — FIXED 0f991d6 (rivers 3)

### B-14 White outline around lakes at high altitude — FIXED 0f991d6 (rivers 3)

### B-13 Waterfall aeration paints the whole downstream reach white at range — FIXED 0f991d6 (rivers 3)

### B-9 Razor water cutoff at mesh-LOD borders — FIXED (Track B merge)
Fixed in the rivers pass (9399ac7): water coverage now degrades
gracefully across LOD levels; the exact time-pinned waterbug-repro.play
capture and a unit test carry the proof. The untimed baseline framing
pose stayed byte-identical as the control.

### B-12 Long settle: blurry landscape took 3+ s to sharpen — FIXED
Two mechanisms (2026-07-16, commit pending): (1) TELEPORT BURST — a
>5 km single-frame camera jump opens stream_caps(2) EAGER for 150
frames (renderer.rs burst_until_frame); the previous scene is gone and
the new pose's refinement ladder is the only work that matters. STRICT
(F9 level 0) never bursts — it stays the faithful v2 reference; kill
switch TRI_NO_BURST=1 for A/B. (2) EASE INHERITANCE — a child tile
landing while its parent is still mid-ease joins the parent's 18-frame
settle window instead of restarting it (inherit_ease), so an area
blurs once, not once per LOD rung. Evidence scripts/b12-settle.play
raw timelines: pixel-settled by frame ~60 with burst vs ~120 without
(both improved from the reported 3+ s by mechanism 2). The remaining
~1.5 s first-frame synchronous cover build at teleport is a separate
pre-existing cost (measured identical in STRICT) — candidate for the
banked never-block pass. Settled captures bypass both mechanisms
(byte-determinism unaffected; reels clean).

### B-10 Eviction storms: vanishing tiles/staircases/bald patches (3cc26ec)
Austin's tripwire console lines + the soak probe convicted the VRAM
budget enforcer: crossing BUDGET purged down to RETAIN in ONE frame
(128 MB mass-vanish), fast multi-area sessions blew past HARD (675
tiles at 1178 MB in the probe), and when the recent set alone brushed
HARD, equal-recency ties dropped RANDOM VISIBLE tiles (the raw_2
staircase). Fixed: proportional continuous shedding in live frames, a
pressure valve when the 180-frame window would force a HARD storm,
and view-angle tie-breaking so forced drops land off-view. The soak
runs storm-free with identical coverage; tripwires stay armed.

### B-11 Altitude motion read flat; catch-up was slow (2026-07-15, b64b5f4)
Two causes, both scheduling: parent stand-ins were used for ALL fresh
uncovered tiles (cheap coarse ones included), and motion throttled
total pending. At balanced/eager the parent dodge now applies only to
the expensive class (level >= 11) and motion bounds only that class.
40 km lateral probe: mid-flight draws 121/109/86/72 -> 508/482/397/376;
strict never converged even at rest (72 vs 538 settled). STRICT (F9
level 0) deliberately preserves old v2 behavior as a comparison
reference. Probe: b11-altitude.play.

### B-4 IMPORTANT: mesh-detail loading perf + ascent lagspikes (2026-07-14)
Fixed by Sol (merge d0fbcab; full measurements in interchange/reviews/
sol-b4-findings-2026-07-14.md). Two causes: (1) fine-LOD impostor
emission spent up to 98.6% of tile CPU enumerating candidates that all
rejected (373k lattice visits to emit zero impostors on one level-11
tile) - two conservative same-face region proofs (barren/shrub
interior; unconditional snow over warp-bounded bilinear extrema) skip
enumeration only when every candidate provably rejects; (2) the
immediate-parent prefetch missed horizon/LOD nodes on fast ascents,
turning them into synchronous urgent builds - the live budget now
checks the deterministic future cover at max(alt+10km, alt*4) first.
First frames 3 km 117.6 -> ~34 ms, 10 km 251.8 -> ~38 ms; tile-cost
gate -38% with identical checksums; moon builds measured and ruled
out. Repro scripts b4-ascent-*.play retained. Residual: B-4a above.

### B-5 Teleport-to-moon: violent rotation jitter at pitch -90 (2026-07-14)
Root cause exactly as ledgered: focus() placed the camera looking
dead-radially at the body, storing pitch = exactly -90 where look is
parallel to radial up and the view basis degenerates (the right-vector
fallback flip-flops under mouse noise). Every INPUT path already
clamped pitch to +-1.50 rad (~86 deg); the fix makes
set_world_orientation enforce the same bound (camera::MAX_PITCH_RAD,
now shared by all clamp sites), so the pole is unreachable from any
path. Focus placements settle at the mouse-reachable bound;
camera-controls.play alignment pins updated to sin(1.50) = 0.99749.

### R-1..R-4 + half of R-5: MP1 internet hardening (2026-07-14)
All from the 2026-07-14 cross-review (fresh Claude context reviewing
Sol's MP1; reports in interchange/reviews/). Fixed same day:
- R-1 slow-client OOM: outbound queues bounded (256) with kick-on-
  overflow (Notify) and a 20 s write deadline so a stalled TCP peer
  cannot park the connection loop.
- R-2 edit flood: per-connection token bucket (30/s sustained, burst
  90; over-budget edits dropped with an edit_rate Error, no disk work)
  + snapshots debounced to a 5 s flusher task (journal stays fsynced
  per edit; EDT1 snapshots are derived caches, final flush on quit).
- R-3 inbound message cap 128 KiB via WebSocketConfig (was ~64 MiB).
- R-4 torn journal tail self-recovers: open() truncates the incomplete
  final record loudly and continues (regression test added).
- R-5a constant-time token compare; R-5b dead import_legacy_if_empty
  removed. R-5c (token in URL query) remains OPEN above.
Verified: 12/12 multiplayer tests; scratch server on 7799 (join, edit
persisted, snapshot flush); production join through the tunnel.

### F-23 River shoreline dead-band pits + the river bay/apron family
Austin photographed dry blocks sunk below the water surface against a
river channel (0.630 69.024). Two stacked causes, both fixed: (1) the
flood test's 20 cm epsilon (`wl > h + 0.0002`) left columns carved to
within 20 cm of the waterline dry, while block quantization renders them
a FULL block below the water top — analytic hairline, visible pit. Flood
tests are now block-quantized (ground block below water block => water).
(2) Rivers had no equivalent of the lake aprons: natural dips below the
waterline just outside the carve stayed dry pits (under the census's 2 m
wall threshold — caught by eye). A bounded bay band floods them (natural
irregular bays, free aesthetics) and a 3% bank apron floors the ground
beyond, mirroring the lake solution. Planet census: total findings
140,873 -> 40,753, WALL 138.7k -> 39.6k, JUMP halved, LIP still 0.
SEAJUMP 25 -> 34: nine new mouth-area film-vs-sea sites, magnitude
small, noted for the next sweep.

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
