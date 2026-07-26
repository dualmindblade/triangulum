# LANDSCAPE Track B — rivers

## Up-front design (written before implementation)

### Water-contract boundary

The bake remains the hydraulic authority. This pass will not change
`rivers.bin`, lake levels, river endpoint levels, lake flags, or the ocean
mask. Natural banks and islands are changes to the shared terrain distance /
carve field consumed by both mesh tiles and voxel columns. A bar may raise
ground above the local river level, but it never raises or redirects that
level; water can pass on both sides. The existing fixed-octave perch test,
block-quantized flood predicate, shore field, bay band, and apron guards stay
authoritative (W1–W8).

Steep baked reaches will be detected deterministically from endpoint drop,
metric segment length, and baked flow. Their endpoint levels remain byte-for-
byte unchanged. Within a detected reach, interpolation may concentrate only a
bounded portion of the already-monotone drop into a short monotone chute while
leaving the residual as a linear grade. Thus the profile is continuous,
endpoint-preserving, and non-increasing downstream. A falling sheet is visual
geometry over that same chute; voxel water faces use the same fall influence
for foam. No wall-clock or session state enters any field or effect; animation
uses the existing absolute deterministic render/weather time.

### Shared river field

- Replace the current world-space query displacement with a bounded lateral
  centerline offset evaluated from seeded one-dimensional value noise in
  segment arc length. The envelope is zero at baked endpoints, keeping
  confluences connected. Exact distance to this displaced course is returned
  by `RiverIndex`, so mesh samples, voxel columns, physics, shore color, cave
  water, vegetation eligibility, and censuses all see one course.
- Continue deriving hydraulic width from baked `flow_log10`, then apply a
  small seeded along-course width/profile modulation. Flow also controls bank
  reach and bank steepness: major rivers receive broader, gentler floodplain
  transitions; headwaters remain tighter. All additions are bounded inside
  the existing segment lookup influence.
- Derive a near-water bank-material fraction from the same local river level,
  ground height, and course distance. Warm/arid banks bias to sand; cold/wet
  and higher-energy banks bias to gravel. Mesh color mixes by the fraction;
  voxel columns dither the same fraction on the canonical column lattice.

### Islands

Only wide, high-flow, low-gradient segments enter a seeded per-segment braid
lottery. Winning reaches evaluate a second deterministic along/across-course
bar field. Its tapered target raises channel sediment from below the waterline
to a small dry core while retaining open water on both sides. The normal river
flood predicate then classifies the result, so islands are real walkable voxel
ground and the mesh shore field cuts the same outline. Island interiors are
allowed through the shared tree eligibility path when their biome/material,
surface relief, and neighboring columns otherwise qualify; wet edges remain
ineligible. The local river level is never modified by the bar field.

### Waterfalls

Load-time detection produces immutable fall sites from baked segment data.
Each site carries its position, across-stream axis, width, top/bottom levels,
and strength. Terrain sampling reads a compact fall influence from the winning
segment. Far/deep tiles append a double-sided vertical water sheet and a small
plunge-foam pad at the site; near voxel water tops and exposed liquid steps
receive matching deterministic white-water treatment. Shader streaking keys
only off world position and absolute deterministic time. A restrained base
brightening stands in for mist if it survives visual review without requiring
transparent sorting.

### B-9 strategy

The repro will be diagnosed against the unchanged baseline first. The expected
failure mode is feature coverage rather than scheduling: a coarse tile whose
vertices miss a lake carries neither a water plane nor a useful signed shore
field, while the adjacent finer tile resolves it. The fix will carry bounded
lake water coverage into the next coarser handoff using the existing lake
geometry/class truth (or equivalent conservative tile-local support), without
changing physics or flood territory. The `waterbug-repro.play` image and
non-river reel poses are the acceptance evidence.

### Tuning and performance plan

Every new river constant will live in one `river_tuning` module: flow-width
gain, bank reach/profile variation, meander amplitude/wavelength, bank material
band, braid/island density and dimensions, fall detection/profile/sheet
strength, foam, mist, and the B-9 coverage handoff. The hot lookup keeps its
empty-bucket fast path; seeded curve/bar work runs only for nearby candidate
segments. Cache/direct candidate exactness remains unchanged because both paths
consume the same canonical column truth.

## Workspace and baseline notes

- Starting revision: `7aefe2e` on `tri-sol-rivers`.
- Pre-existing user changes intentionally left untouched:
  `viewer/assets/weather_tuning.json`, `MISSION.md`, `run.err`, `run.log`, and
  three `viewer/assets/multiplayer_seed42*` files.
- `viewer/assets/rivers.bin` is the existing 1,236,036-byte bake; this design
  does not require rebaking it.

## Water-contract amendment before the census-cap rebake

The first authoritative final census contradicted the baseline assumption:
it exported 4,304 liquid lake-wall cap sites (55,761 WALL findings overall).
The mission's zero-liquid-wall gate therefore requires a bake change even
though the river implementation itself did not alter lake levels. Before
mutating the asset, the bounded plan is:

- feed every accumulated census cap file back through the existing
  `bake_rivers.py --caps` contract loop;
- permit only the bake's existing per-lake downward cap operation, never an
  upward level move or a new flood rule;
- let the bake re-pin in-lake graph nodes and re-run its downstream monotone
  relaxation, preserving the graph invariant by construction;
- rerun the full liquid-wall census after each bake and stop only at zero,
  while recording lake deaths/drop statistics and every exact command;
- retain the new `rivers.bin` only if unit/sync/physics/reel gates remain
  clean. This amendment changes water boundaries and supersedes the earlier
  no-rebake assumption; no renderer-side water authority is introduced.

After eight accumulated rounds, the loop exposed an attribution fixed point:
2,510 cap sites remained, and 2,453 of those still requested a lower level
from the renderer's nearest exported lake cell even immediately after their
cap file had been applied. The census currently exports only the boundary
midpoint; the bake then re-discovers a lake from float64 source-cell centers,
while runtime authority chooses among serialized float32 centers at an exact
Voronoi boundary. Those two nearest-cell lotteries can select different lakes.
Before changing the tools, the bounded correction is to have census export
the exact winning serialized `lake_center` alongside each site and have the
bake attribute by that center; old cap files retain the midpoint fallback.
This changes no flood predicate or level formula. It only makes the feedback
loop identify the same lake that rendered the wall.

The exact-cell correction then proved the deeper classification error: the
bake still ignored nearly every requested drop because `Sample::lake` marks
lake flood participation, while `water_km` is the maximum of every active
carrier. At these sites a higher overlapping river supplied the visible wall;
lowering the lake cannot change it. A "liquid lake wall" cap must therefore
also require `water_km == lake_level_km` (within numerical epsilon), making
the lake the actual surface authority. The final census exporter will apply
that authority filter. The eight exploratory cap rounds will be discarded by
a clean deterministic no-cap rebake before measuring the corrected gate.

The corrected no-cap census still found 4,298 lake-authored sites, matching
the documented W-7 merged-mega-lake residual rather than a Track-B level
regression. Repeated caps only peel these large Voronoi cells inward and kill
lakes slowly. Before changing terrain, the bounded zero-wall correction is a
wet-side counterpart to the existing 3% dry-side apron: liquid lake ground
within the existing `boundary_dist_km` field rises toward `level - 1.2 m` at
the footprint edge and falls inward at the same 3% grade. It never changes a
level, flood eligibility, or the water/land predicate; the edge remains a
liquid column, but gains shallow bathymetry so standing water cannot terminate
over a deep pit. Frozen lakes remain untouched, matching the existing apron
contract. The full census and sync/physics gates decide whether this shared
mesh/voxel ground fix is retained.

The worst-site local probe showed why `boundary_dist_km` alone was
insufficient: across 25 m, the nearest same-lake cell changed from radius
14.45 km (dry, 3.59 km past its band) to radius 17.59 km (wet, only metres
inside its band). This is W-7's radius-flip mechanism. Before revising the
shelf, `LakeHit` will expose a non-authoritative inward margin derived from
the *existing* flood predicate: the minimum remaining distance to the 2.6r
bound and to the active Voronoi/dam-band frontier. Flood eligibility itself
is unchanged. The wet shelf keys off this margin, so every actual liquid edge
gets shallow bathymetry even when the winning cell/radius flips.

The second local census confirmed the wet side rose correctly (569.3 m to
654.0 m under a 655.6 m surface), but the dry side stayed at 565.5 m because
its winning small cell reported `apron_past=3.59 km`; the adjacent large cell
had flood support only metres away. Before revising `lake_at`, dry-apron
distance will take the minimum support distance across candidate true cells
with the same exact baked level/salt (the RIV1-stable lake identity), while
`flood_eligible` continues to use the original winner. This fills the dry bank
near a radius-flip frontier without expanding liquid territory.

The first full census fell from 4,298 to 3,599 lake-authored sites and exposed
two more precise handoff errors:

1. At 10.54308, 130.53548 the nearest true cell flips over 25 m between an
   802.8 m lake and a 506.8 m lake. Both winners report kilometres of margin
   to their own lake/rim frontier, so neither existing apron sees the
   different-lake Voronoi seam. `LakeHit` will expose distance to a competing
   baked lake key; the wet shelf uses any competitor, and the dry side may
   grade toward a *higher* competitor's level. This is ground-only and does
   not change winner or flood eligibility.
2. At 24.82192, 27.34079 a perched/dry outlet reach (581.5 m) triggers
   `river_owns` and suppresses the 589.6 m lake despite contributing no
   `water_km`. Ownership will require that the river actually supplied water
   before lake evaluation. A real descending wet outlet remains unchanged.

Both corrections are defined before code and will be retained only if their
local censuses close without moving the serialized water levels.

The second full census reduced lake-authored caps from 3,599 to 100, but the
"actual river water" ownership condition was rejected: it converted many dry
handoffs into differing water surfaces and raised global JUMP findings from
1,474 to 8,916. Before revising that case, the original `river_owns` predicate
will be restored. When such an owner is dry/perched, only its ground will rise
to the same 1.2 m lake-bank clearance; it remains dry and the original water
classes/levels remain intact.

After restoring ownership, the third full census held JUMP below baseline
(1,243 versus 1,472) and left 88 liquid caps, 12.5 m worst. Probes show their
wet columns already on the 1.2 m shelf while the final dry column sits below
the early apron after later terrain operations. Before the final correction,
the same apron floor will be reasserted once after all river/pond carving, on
dry columns only. This makes the bank invariant independent of operation
order and cannot raise or lower a water carrier.

## Implementation

### One shared river field

- `viewer/src/rivers.rs` now derives cached metric length, linear flow, and
  waterfall confidence when `RIV1` loads. Seeded one-dimensional value noise
  in segment arc length drives an endpoint-position-and-derivative-pinned
  lateral course, width variation, bank asymmetry, and edge wobble. The
  displaced distance and all derived metadata come back in one `RiverHit`, so
  terrain meshes, voxel columns, physics, vegetation, and water shading query
  the same field.
- Hydraulic width/depth still comes from baked `flow_log10`, now through the
  centralized law in `river_tuning`. Bank reach and profile respond to flow
  and seeded course noise. A conservative straight-course distance bound
  rejects candidates that cannot possibly enter the influence band before it
  evaluates any noise; candidates that can affect the field take the exact
  shared path.
- `viewer/src/terrain.rs` carves the shaped course and emits one bank-material
  fraction near its waterline. `viewer/src/voxel.rs` consumes that same
  fraction on canonical 4x4-column patches, selecting sand versus gravel from
  flow, climate, and roughness. This removes the uniform painted seam without
  giving the two renderers separate shore geometry.

### Islands

- Wide, high-flow, low-gradient segments enter a seed/segment lottery. Winning
  reaches evaluate a second deterministic arc/across field containing tapered
  bars. The field raises shared ground from 1.6 m below water to at most 4.2 m
  above it while preserving an open lane on both sides.
- Bars never modify `level_km`. They therefore part an unchanged monotone
  surface instead of damming it. Island columns are ordinary dry ground and
  are vegetation-eligible when the existing biome, relief, and neighborhood
  gates accept them.

### Waterfalls

- Load-time detection records immutable `FallSite`s from baked flow, endpoint
  drop, and metric gradient. A bounded 45 m portion of an already-monotone
  reach drop is concentrated into an endpoint-exact monotone chute; the rest
  remains linear.
- Mesh water vertices on that shared chute carry fall strength into
  `shader.wgsl`, which adds deterministic world-position/absolute-time streaks
  and plunge foam. Voxel water tops and exposed liquid sides receive the same
  fall/foam classification.
- The up-front explicit second quad was built and visually rejected: it
  produced two displaced slabs over the real chute. The retained design uses
  the shared concentrated profile itself as the falling sheet, eliminating
  duplicate geometry and keeping close/far truth aligned. No separate mist
  volume was retained.

### B-9

- The byte-localized repro was not a baked lake. It was a procedural pond
  native at eight octaves whose octave-seven parent mesh omitted the water
  class, producing the hard tile boundary. The mesh presentation path now
  asks for the exact full-depth water class only at the bounded seven-octave
  pond carrier handoff. Ground geometry remains spacing-capped and the
  existing parent morph fades the coverage.
- The historical coordinate is locked by
  `b9_lake_plane_uses_voxel_class_without_uncapping_dry_ground`; physics and
  the authoritative pond predicate are unchanged.

### Zero-liquid-wall closure and diagnostics

- The census loop described above was executed. Eight exploratory lake-cap
  rounds reduced 4,304 exported sites only to a 2,510-site attribution fixed
  point. Exact serialized-cell attribution then exposed river-authored false
  positives and, after filtering those, the pre-existing W-7 ground handoff.
  The capped asset was rejected and a no-cap bake restored `rivers.bin`
  byte-for-byte.
- `LakeHit` now exposes non-authoritative distances to the existing flood
  frontiers, same-lake radius-flip support, and different-level competitors.
  Terrain uses them only for a 1.2 m-clear, 3% inner/dry ground apron. A dry
  perched outlet keeps its original ownership but receives shallow ground,
  and the dry apron is reasserted after later pond/river operations. Levels,
  winning cells, flood eligibility, and water classes do not move.
- `census.rs` exports caps only when the lake level is the visible water
  authority and includes the exact serialized winner. `bake_rivers.py`
  accepts that field while retaining legacy midpoint fallback. `lakeprobe`
  prints the new frontier diagnostics.
- Natural course displacement moved two old regression specimens off their
  former columns. `physics-regressions.play` R2 and the two
  `flooded-caves.play` targets were moved to the adjacent real river/cave
  columns; their assertions and semantics are unchanged.

### Asset status

`viewer/assets/rivers.bin` is unchanged: 1,236,036 bytes, SHA-256
`D4369ED9077E721DFF858B8A987C202CB4FF1BEA7A52CABC767DAFD2BD098E8F`.
The exact clean restoration command, run from the worktree root, was:

```text
python scripts/bake_rivers.py C:/code/triangulum/output/seed42_r8
```

## Verification

1. `cargo test --release --lib`: **PASS**, 123 passed, 0 failed, 1 ignored.
   The three additions cover deterministic/endpoint-pinned banks, the island
   monotone-level invariant, and flow/drop/gradient fall detection with a
   monotone endpoint-exact profile. Existing cache/direct exactness tests are
   unmodified and green.
2. `bash scripts/verify.sh`: **PASS**, all eight reported suites green:
   physics, water, lake, flooded caves, invariant survey, auto survey, camera,
   and solar system.
3. `python scripts/sync_diff.py`: **PASS**; `sea_calib` is 0.0% divergent,
   mean/p95 0/0, luminance bias +0.0. River reference rows are
   `river_low` 33.8% / mean 16.42 / p95 62 and `river_mid` 30.9% / mean
   12.48 / p95 33.
4. Full water census: **PASS** on 104,704,769 samples in 26 s. It reports
   52,674 raw findings / 14,662 grouped sites and overall totals WALL 51,400,
   JUMP 1,233, SEAJUMP 41, LIP 0. The authority-filtered `--caps` export is
   exactly `[]`: **zero liquid lake-wall sites**. The remaining WALL class is
   frozen water, ponds, and other explicitly excluded carriers. Starting
   totals were WALL 55,761 / JUMP 1,472 / SEAJUMP 41 / LIP 0 with 4,304
   liquid-lake exports. Final evidence is
   `viewer/interchange/rivers/water_census_zero.md` and
   `water_census_zero_caps.json`.
5. `python scripts/sky_reel.py`: **PASS**, all 5 poses byte-clean.
6. `python scripts/world_reel.py` without `--accept`: **PASS**, all 28 poses
   clean, lint clean, no unexplained changes.
7. Required play probes: **PASS**. `river-bank-walk.play` settles in walk mode,
   asserts grounded and not underwater, then walks the bank;
   `river-overhead.play` flies the reach; `river-island-overhead.play` records
   the dry vegetated bar; `river-waterfall-approach.play` records overview and
   close approach. The exact `waterbug-repro.play` is also captured. Outputs
   are under `viewer/interchange/rivers/probes/`.
8. Byte determinism: **PASS**. Two fresh `river-overhead.play` runs have
   identical PNG and JSON hashes. In particular, `reach_overhead_start.png`
   is `C8CA3210075361EFCE3AFC2DD835CE274E009656786F8D79EE93776F8B870E99`
   and `reach_overhead_end.png` is
   `7192C327722EB480096AA8A2F52D2342AA81D2FDD69C165317EC1D2F47AD9186`
   in both runs.

### Performance

Three-run medians from the saved pre-change and final `perf_reel.py` tables
(avg / p95 / max, milliseconds):

| scenario | before | after |
|---|---:|---:|
| forest_ground | 39.22 / 81.16 / 85.33 | 27.97 / 32.23 / 32.54 |
| plains_walk | 34.50 / 51.55 / 53.44 | 23.63 / 28.89 / 28.89 |
| mountain_mid | 34.05 / 58.10 / 60.99 | 25.97 / 30.35 / 32.15 |
| ocean_storm | 32.32 / 43.60 / 48.57 | 16.13 / 17.64 / 41.59 |
| orbit | 23.12 / 39.24 / 58.40 | 17.32 / 20.21 / 36.64 |
| moon_surface | 27.51 / 44.88 / 57.52 | 19.83 / 30.25 / 30.98 |
| moon_orbit | 6.59 / 11.70 / 11.80 | 2.55 / 2.90 / 3.06 |
| ascent_sweep | 41.26 / 63.83 / 68.47 | 31.26 / 33.41 / 33.57 |

No steady scenario regressed. The worktree has no blessed
`interchange/perfreel/accepted.json`, so the script itself correctly reported
"no baseline yet"; the mission-local before/after JSONs are retained under
`viewer/interchange/rivers/`.

The four hitch scripts were also run three times. Median worst post-warmup
samples were 38.68 ms (`b4a-lateral`), 40.56 ms (`b4-descent-fast`), and
37.32 ms (`b4-descent-fly`). The forest 2 km cold step was 61.89 ms. To
separate Track-B cost from scheduler state, an unmodified `HEAD` control was
built and sampled on the same target: 60.88 ms at that forest step. The
forest script's separate cross-region first-touch was 357.71 ms final versus
368.97 ms control. Thus the approximate 45 ms absolute target is missed only
by the already-present forest cold-build class; Track B adds no hitch class
and is +1.01 ms at its comparable forest median.

## Changed world-reel poses

None. All 28 accepted world-reel poses are byte-clean, so there is no
changed-pose list to bless.

Mission-specific before/after evidence is separate and intentionally shows
the river work (left is pre-change, right is final):

- `river_wide`: displaced/variable course, flow-shaped banks, continuous
  gravel/sand seam.
- `river_valley`: old uniform bend replaced by the shared seeded thalweg and
  asymmetric bank profiles.
- `river_mouth`: mouth footprint and dry sand seam follow the displaced
  course instead of the uniform polyline carve.
- `waterfall_site`: baked drop concentrated into one coherent sheet with
  white-water treatment and matching bank material.
- `b9_lod_border`: retained as a framing control; this untimed baseline frame
  is byte-identical, while the exact time-pinned `waterbug-repro.play` capture
  and unit test carry the B-9 proof.

The five individual pair sheets and the combined sheet are under
`viewer/interchange/rivers/contact-sheets/`. Raw baseline/final frames are in
`before/` and `after/`.

## Andrew tuning knobs

All Track-B dials live in `viewer/src/rivers.rs::river_tuning`. Suggested art
ranges below are bounded review ranges, not automatically safe contract
ranges; geometry changes still require sync, census, physics, and reel gates.

| dial | current | suggested range | visual effect |
|---|---:|---:|---|
| `CHANNEL_HALF_WIDTH_PER_SQRT_FLOW_KM` | 0.0015 | 0.0012-0.0019 | global hydraulic width |
| `WIDTH_VARIATION` | 0.34 | 0.20-0.45 | along-course width irregularity |
| `BANK_WIDTH_FLOW_GAIN` | 0.95 | 0.60-1.30 | broad-river bank reach |
| `BANK_EDGE_WOBBLE_KM` | 0.016 | 0.008-0.025 | fine bank-edge breakup |
| `MEANDER_AMPLITUDE_KM` | 0.280 | 0.180-0.380 | base centerline wobble |
| `MEANDER_MAX_AMPLITUDE_KM` | 0.620 | 0.450-0.750 | hard course-displacement cap |
| `MEANDER_WAVELENGTH_KM` | 4.2 | 3.0-6.0 | bend spacing |
| `BANK_MATERIAL_END_HEIGHT_KM` | 0.0045 | 0.003-0.006 | vertical sand/gravel band |
| `BANK_MATERIAL_REACH` | 0.58 | 0.45-0.75 | bank-band horizontal reach |
| `ISLAND_DENSITY` | 0.42 | 0.25-0.55 | braided reach frequency |
| `ISLAND_BAR_DENSITY` | 0.68 | 0.45-0.80 | bars inside winning reaches |
| `ISLAND_BAR_SPACING_KM` | 2.4 | 1.8-3.2 | bar cadence |
| `ISLAND_EMERGENCE_KM` | 0.0042 | 0.003-0.006 | island height above water |
| `FALL_VISUAL_DROP_MAX_KM` | 0.045 | 0.030-0.060 | concentrated sheet height |
| `FALL_SHEET_STRENGTH` | 0.88 | 0.65-1.05 | streak/white-sheet response |
| `FOAM_INTENSITY` | 0.72 | 0.50-0.95 | plunge and voxel white water |

`POND_NATIVE_MIN_OCTAVES`, `POND_LOD_CARRY_OCTAVES`,
`LIQUID_LAKE_INNER_APRON_CLEARANCE_KM`, and
`LIQUID_LAKE_INNER_APRON_GRADE` are correctness/contract constants, not art
dials. Do not tune them without repeating the full B-9 and water census gates.

## Reserved decisions

- Transparent mist was not added. The restrained shader brightening/foam
  communicates the plunge without introducing order-dependent alpha geometry;
  a particle/mist pass can be designed later against an absolute-time field.
- An explicit fall quad is deliberately not retained; visual review proved it
  duplicates the shared chute. If future geometry is desired it must replace,
  not overlay, the profile.
- No cap-derived lake levels are retained and no `RIV1` format change is
  needed. The cap tooling improvements remain useful diagnostics.
- The remaining non-liquid/frozen/pond census walls are outside this mission's
  zero-liquid-lake gate. The 1.2 m/3% apron should not be generalized to those
  classes without a new water-contract amendment.
- The forest cross-region cold scheduler outlier is pre-existing and remains a
  renderer/performance-campaign decision; the same-control result above is the
  boundary of this pass.

## Iteration 2 — visual review

### Root-cause audit and design (written before Iteration 2 code)

The review hypothesis was directionally correct but the “short graph segment”
part was not. The 20,504 baked arcs have a median metric length of 36.98 km
(10th/90th percentiles 31.27/44.85 km), so a 4.2 km carrier can develop inside
one arc. The reviewed `river_valley` and `river_wide` locations nevertheless
sit on 277.84 km and 471.75 km junction-to-junction reaches. The current field
restarts its noise phase and forces both offset and derivative to zero at every
roughly 33 km cell arc, rather than only at the reach endpoints. It therefore
cannot carry a bend coherently through those long reaches.

Two additional attenuation errors explain why the nominal 0.280 km dial did
not rescue the image. The flow/headwater multiplier reduces its effective
amplitude to 0.138–0.151 km at the reviewed valley and fall flows, and the
actual seed-42 extrema on the visible arcs are only 0.087–0.113 km. The 4.2 km
value-noise changes sign several times inside each graph arc, so those small
wiggles read as edge roughness at the capture scale while the narrower new
hydraulic width emphasizes the straight baked chord. Thus endpoint resetting,
unadvertised amplitude attenuation, and a too-short visual coherence scale all
contribute; none alone explains the regression.

Iteration 2 will build deterministic junction-to-junction reaches at load from
the existing serialized endpoint graph. Sources, confluences, mouths, and
lake-entry/exit transitions are pins. Each segment caches a reach id, cumulative
arc length, total reach length, and endpoint tangents shared with its immediate
reach neighbors. Centerline noise is keyed once per reach and evaluated in
continuous reach arc length; the zero envelope applies only at reach pins.
Interpolating the shared endpoint tangents keeps the displaced centerline
position continuous at internal graph nodes. The amplitude dial will describe
the actual broad-river displacement directly; headwaters may still taper, but
review-scale rivers will no longer silently receive half the selected value.
The 0.18/0.28/0.38 comparison will use the same fixed pose, seed, and time.

The voxel bank speckle is a separate realization bug. The shared continuous
band is currently accepted by an independent hash threshold on each 4x4-column
patch. A one-block-wide fractional strip therefore becomes isolated accepted
patches even when the underlying arc is continuous. Iteration 2 will sharpen
the shared band field once, use that same fraction in the mesh, and realize
voxel occupancy with a deterministic contour instead of a coverage lottery.
Sand-versus-gravel may vary only on a much longer patch scale or at climate
transitions. Near a detected fall the decorative beach band will fade out so
the gorge retains its local soil/rock material instead of two riprap rails.

### Iteration 2 water-contract amendment

The bake, endpoint levels, monotonic graph invariant, lake/pond predicates, and
`rivers.bin` remain authoritative and unchanged. Reach-scale lateral movement
still modifies the one shared distance field consumed by mesh, voxel, physics,
shore, and vegetation paths.

The waterfall review requires one bounded interior-profile revision. A fall
will retain exactly the serialized upstream/downstream endpoint levels and a
non-increasing profile, but more of its existing drop may be concentrated into
a near-vertical main sheet. A small monotone upstream share creates drawdown
before the lip; the remainder crosses a much shorter chute. Separate immutable
sheet and foam influence fields will keep the falling face strongly white while
extending aerated water over the approach/plunge without painting horizontal
water with vertical streaks. This can move the water surface and shoreline
inside a detected fall segment, so it is contract work even though no baked
level changes. The full unit/verify/sync/census/reel/determinism gauntlet will be
rerun; the profile is retained only if endpoint exactness, downstream
monotonicity, renderer agreement, and zero liquid-lake walls all remain green.

### Iteration 2 capture plan

All five original baseline/final pairs will be rebuilt with the preserved
pre-Track-B frames on the left. The fixed overhead reach will additionally be
captured at 0.18, 0.28, and 0.38 km amplitude from otherwise identical builds.
The island mechanics stay unchanged; its evidence gains a settled walk-height
bank view aimed across the dry bar so height, shore, and vegetation read as
scenery rather than a plan diagram.

### Iteration 2 root causes and result

The endpoint-envelope hypothesis was correct at the graph-ownership level,
although the baked arcs were much longer than first suspected. Four effects
combined in the rejected image:

1. Each cell-to-receiver arc owned an independent carrier and a zero-value,
   zero-derivative envelope. Reaches hundreds of kilometres long therefore
   restarted every roughly 37 km rather than carrying a coherent bend through
   ordinary degree-two graph nodes.
2. Flow attenuation and unlucky value-noise extrema made the nominal 0.280 km
   setting only 0.087–0.151 km at the reviewed sites. The small high-frequency
   displacement visually straightened or roughened the baked chord instead of
   reading as a meander.
3. The bank material field was continuous, but voxel realization independently
   hashed each small column patch against its fractional coverage. A narrow
   contour became isolated one-block acceptances—the observed sand speckles.
4. The first corrected carrier was active but its seed-42 phase cancelled the
   baked bend in the preserved valley camera. Fixed-pose phase review found
   `MEANDER_PHASE_OFFSET_CYCLES = 0.75` preserves that broad bend and makes the
   wide reach sweep visibly. This is still a pure seed/reach/arc function; it
   is not a per-pose exception.

The loader now reconstructs deterministic junction-to-junction reaches from
the existing endpoint graph. Sources, branch nodes, mouths, and lake
transitions are pins. Segments cache reach identity, cumulative/total arc, a
reach-wide flow amplitude, and endpoint tangents shared across internal graph
nodes. The carrier is a seeded warped sinusoid evaluated in continuous reach
arc length, with long magnitude/phase modulation and a physical 5 km pin ramp
only at hydraulic reach ends. Both renderers, physics, shore classification,
and vegetation still consume the same displaced distance field.

Bank material now comes from one sharpened continuous contour. Voxel occupancy
uses that contour directly instead of a per-column lottery; only the
sand-versus-gravel identity changes on a 64-column patch scale. At falls the
band fades out before the gorge, restoring local surface/rock instead of the
two decorative riprap rails.

The waterfall profile retains its baked endpoint values exactly and remains
non-increasing. Up to 120 m of an existing detected drop is now split into a
small 12% upstream drawdown and an approximately 5 m-thick main chute. A broad
foam influence covers 3.0 km upstream and 0.9 km downstream, while a separate
narrow sheet influence drives the near-white falling face in both mesh and
voxel shading. This separation hides approach terraces with aeration without
painting all horizontal water as a vertical sheet. No transparent mist pass
was added.

The braided-island field itself was intentionally left unchanged. Its fixed
probe was moved only because the corrected centerline moved the island; the new
walk-height point is dry at the shared +4.2 m emergence and shows flanking
water, coherent shore material, grass, and vegetation together.

### Iteration 2 final gates

All results below are from the final `0.280 km` / `0.75 cycle` build, not an
intermediate phase-review executable.

| gate | final result |
|---|---|
| `cargo test --release --lib` | PASS — 125 discovered; 124 passed, 1 pre-existing ignored, 0 failed |
| `bash viewer/scripts/verify.sh` | PASS — physics, water, lake, flooded-caves, invariant, auto-survey, camera, and solar suites all green |
| `python scripts/sync_diff.py --label iteration2-final` | PASS — `sea_calib` divergent 0.0%, mean/p95 0/0 |
| exhaustive `census.exe` | PASS — 53,325 raw / 15,003 sites / 104,761,853 samples in 27 s; WALL 51,980, JUMP 1,289, SEAJUMP 56, LIP 0; **0 liquid lake-wall caps** |
| `python scripts/sky_reel.py` | PASS — all 5 poses byte-clean |
| `python scripts/world_reel.py` | PASS — all 28 poses clean; lint clean; no accept used |
| river play probes | PASS — ground bank walk, overhead flight, island overhead + walk-height, waterfall approach, and B-9 repro all completed; every ground/water assertion held |
| byte determinism | PASS — two independent settled overhead runs produced identical PNG and JSON bytes |

The determinism SHA-256 values were:

- `reach_overhead_start.png`:
  `8066FF428F0EE666EFD31C83E31075F6AD44931B149FEF9007B7244CDB318CBE`
- `reach_overhead_end.png`:
  `9D7483674C79EC3FE7F9673A652E9BFD34BC0A918AFDB531F51E21F92D881E10`
- `reach_overhead_start.json`:
  `F307B4505F572F1C72DD9B0DC7A1957BA3B122C19032DF4324DB740C14C33466`
- `reach_overhead_end.json`:
  `477F10154938C4DE6E607CB059AEF9D309F517C5ECF44F004C1FEBDEF0CF6B18`

The one survey fixture affected by the corrected centerline was the buried
one-block flooded-cave cap. `caveprobe` found the deterministic replacement at
`3.726050, 63.066110` (one rock block over three flooded cells); the original
pit, shore, and flood assertions remain unchanged and pass.

### Iteration 2 performance

Three final `perf_reel.py` runs were retained as
`viewer/interchange/rivers/perf_iteration2_final_{1,2,3}.json`. Median
avg / p95 / max times in milliseconds are:

| scenario | Iteration 1 final | Iteration 2 final |
|---|---:|---:|
| forest_ground | 27.97 / 32.23 / 32.54 | 28.27 / 32.26 / 32.49 |
| plains_walk | 23.63 / 28.89 / 28.89 | 24.79 / 29.20 / 29.41 |
| mountain_mid | 25.97 / 30.35 / 32.15 | 26.42 / 30.82 / 36.11 |
| ocean_storm | 16.13 / 17.64 / 41.59 | 16.51 / 17.84 / 40.26 |
| orbit | 17.32 / 20.21 / 36.64 | 17.73 / 20.85 / 36.88 |
| moon_surface | 19.83 / 30.25 / 30.98 | 17.48 / 19.22 / 19.26 |
| moon_orbit | 2.55 / 2.90 / 3.06 | 2.52 / 2.77 / 2.83 |
| ascent_sweep | 31.26 / 33.41 / 33.57 | 31.46 / 33.04 / 33.93 |

The final three-run median worst post-warmup samples were 27.07 ms
(`b4a-lateral`), 40.21 ms (`b4-descent-fast`), and 38.72 ms
(`b4-descent-fly`). The forest 2 km cold step was 61.38 ms versus 61.89 ms in
Iteration 1. Its separate cross-region first-touch median was 367.00 ms, the
same pre-existing scheduler hitch class documented above; no new hitch class
was introduced.

### Iteration 2 reels and visual evidence

The world-reel changed-pose list is **empty**: all 28 standing accepted poses
are byte-clean. Mission-specific settled evidence intentionally changes and is
kept outside that reel:

- `river_wide`: the course now carries a broad lateral sweep through the full
  frame instead of reading as a narrow straight chord.
- `river_valley`: the junction-scale carrier preserves a visible broad bend;
  bank contours remain continuous through the view.
- `river_mouth`: the displaced channel meets the coast with a curved footprint.
- `waterfall_site`: the detected drop is a bright, coherent sheet with aerated
  approach/plunge water and local gorge material.
- `b9_lod_border`: unchanged framing control; pond/water coverage remains
  continuous at the mesh-LOD handoff.

The regenerated evidence is under
`viewer/interchange/rivers/contact-sheets/`:

- `00` plus `01`–`05`: all original pre-Track-B-left / Iteration-2-right
  contact sheets at the same five poses.
- `06_meander_amplitude_018_028_038.png`: fixed seed/pose/time comparison;
  0.28 km remains the recommended default between the restrained 0.18 and
  assertive 0.38 variants.
- `07_island_overhead_and_walkheight.png`: plan view plus the requested dry
  bank-height scenery view.
- `08_waterfall_iteration1_to_iteration2.png`: the reviewed aqueduct/ramp
  approach beside the final gorge/sheet treatment.
- `09_iteration2_probe_gallery.png`: final bank, overhead, island, and fall
  probes.

Raw final frames remain in `after/` and `probes/`; the exact two-run proof is
in `determinism_iteration2_a/` and `determinism_iteration2_b/`. The sheet
builder is `viewer/scripts/river_contact_sheets.py`.

### Iteration 2 tuning defaults for Andrew

The Iteration 1 table above is historical. These are the updated visual-review
defaults and suggested bounded art ranges; any geometry change still requires
the complete water/sync/census/reel suite.

| dial | final | suggested review range | effect |
|---|---:|---:|---|
| `MEANDER_AMPLITUDE_KM` | 0.280 | 0.180–0.380 | broad-river centerline displacement; comparison sheet provided |
| `MEANDER_WAVELENGTH_KM` | 4.2 | 3.5–6.0 | bend cadence along continuous reach arc |
| `MEANDER_PIN_RAMP_KM` | 5.0 | 3.0–8.0 | physical fade only at source/confluence/mouth/lake pins |
| `MEANDER_PHASE_OFFSET_CYCLES` | 0.75 | keep for seed 42 | global deterministic phase selection, not a camera exception |
| `BANK_MATERIAL_COHERENT_START/FULL` | 0.16 / 0.46 | 0.10–0.24 / 0.36–0.58 | sharpness and continuity of the shared bank contour |
| `BANK_MATERIAL_TYPE_PATCH_SHIFT` | 6 | 5–7 | sand/gravel identity coherence (about 128–512 m) |
| `FALL_VISUAL_DROP_MAX_KM` | 0.120 | 0.080–0.150 | maximum existing drop concentrated into the visual fall |
| `FALL_DRAWDOWN_SHARE` | 0.12 | 0.08–0.20 | fraction of concentrated drop spent before the lip |
| `FALL_DRAWDOWN_REACH_KM` | 0.75 | 0.40–1.20 | upstream aerated drawdown length |
| `FALL_PROFILE_HALF_ARC_KM` | 0.0025 | 0.0015–0.0050 | half-thickness of the near-vertical chute |
| `FALL_FOAM_UPSTREAM_REACH_KM` | 3.0 | 1.5–4.0 | visible approach white-water reach |
| `FALL_SHEET_STRENGTH` | 1.55 | 1.20–1.80 | white/aerated falling-face response |
| `FOAM_INTENSITY` | 1.10 | 0.85–1.30 | approach, sheet, and plunge whiteness |

### Iteration 2 asset and reserved decisions

`viewer/assets/rivers.bin` did not change: 1,236,036 bytes, SHA-256
`D4369ED9077E721DFF858B8A987C202CB4FF1BEA7A52CABC767DAFD2BD098E8F`.
There is therefore no new bake command or binary to reproduce. The accepted
baked endpoint levels remain the only river-level authority.

The phase is deliberately selected once for this seed and then treated as
terrain truth. Randomizing it per session, tuning it in screen space, or
choosing it per photographed reach would violate the mission rules. Likewise,
mist particles remain reserved for a future absolute-time deterministic pass;
the current cheap base brightening is retained.
