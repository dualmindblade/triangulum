# B-17 + B-18 — natural range boundaries, seamless LOD tint

Worktree: `tri-sol-lodcolor`, base `07ddaa3`  
Mission evidence root: `viewer/interchange/lodcolor/`

## Baseline captured before implementation

- Full 112-pose Austin recording:
  `viewer/interchange/lodcolor/before/rec001/frames/`
  - Contract frames: `f000020`, `f000035`, `f000060`, `f000090`,
    `f000111`.
  - `f000035` (3.74078 km) has the reported broad tint/dither step.
- Seven clear orbital candidates:
  `viewer/interchange/lodcolor/before/orbit_candidates/`
  - Selected B-17 specimens:
    - `candidate_l1_north`: literal orthogonal coast steps.
    - `candidate_rec001`: long vertical coast stair.
    - `candidate_inland_sea`: sawtooth/polygonal inland-sea rim.
- L-1 accepted-character pair:
  `viewer/interchange/lodcolor/before/l1_acceptance/`
  - 54.567, 130.853 at 7.852 / 3.647 km.
  - 12.572, 12.619 at 6.615 / 3.893 km.

The baseline command required by the mission was run verbatim:

```text
TRI_POSE_RENDER=1 cargo run --release --example play -- scripts/rec001-replay.play
```

## Diagnosis

### B-17: the reported range grid is not the current range comparator

The narrow suspicion in BUGS.md is stale relative to the code now on main:

- `biome_range_comparator` already evaluates Perlin-style `kgnoise`.
- Its domains already use the three rotated `BIOME_RANGE_AXES`.
- Its octaves already retire at screen Nyquist.

The selected captures show the worst grid on the *physical shoreline*, not on
the already-fractal biome owner. The actual loss occurs in
`terrain::shore_field`:

1. `sea_from_fields` owns the correct continuous inputs (`e_raw`, sharp water
   fraction, blurred ocean fraction), but `shore_field` reduces their result
   to the Boolean `Sample::sea`.
2. The sea side then gets a positive elevation magnitude, the land side a
   negative one, and both saturate at `SHORE_CLAMP_KM` (5 m).
3. A coarse range triangle therefore usually sees an almost binary `-5 m /
   +5 m` carrier. Its fragment zero crossing falls near the triangle/grid
   midpoint instead of where the underlying classifier margin crosses zero.
4. Derivative AA makes that line crisp, but cannot recover discarded
   curvature. The result is the observed orthogonal staircase and long
   polygonal chords.

This is a reconstruction defect. Physical sea ownership, terrain, physics,
and the categorical biome field do not need to move.

### B-18: the expected color is not actually carried through interpolation

The root suspicion is directionally correct, with a more specific mechanism:

1. Every mesh vertex packs eight endpoint RGB values and eight cumulative
   family thresholds.
2. A fragment interpolates both arrays across its ring-specific triangle.
3. It reconstructs `range_mean` by multiplying the interpolated endpoint by
   the interpolated threshold interval and summing:
   `Σ(E_interp × ΔT_interp)`.
4. Interpolation and multiplication do not commute. This creates
   ring-dependent quadratic cross-terms even when every vertex queried the
   same canonical world field.
5. The categorical reconstruction is centered around that derived mean, so
   the error changes both broad tint and apparent dither density. Different
   triangle footprints expose it as a ring step. The 4–8 km range handoff
   then turns the same error into descent tone drift toward the exact local
   mesh/voxel color.

The CPU already computes the intended mean (`target_mean`) before it centers
the categorical endpoints, but that invariant is not sent to the fragment.

## Design fixed before code

### B-17: continuous authority-margin shoreline carrier

Replace only the sea portion of the presentation `shore` carrier with a
continuous signed margin whose sign is exactly equivalent to
`sea_from_fields`:

```text
sea = (e_raw <= 0) AND
      ((water_frac >= 0.5) OR
       ((e_raw <= -0.1) AND (ocean_frac > 0.35)))
```

The signed carrier will compose the same predicate with continuous `min/max`
margins, normalized into the existing ±5 m payload. It retains the analog
distance to all three classifier surfaces instead of collapsing them to a
Boolean sign. The zero set therefore remains class-faithful while coarse
triangles preserve substantially more position/curvature.

Fallback: if the three accepted orbital specimens still show long straight
facets, add a range-only, screen-filtered rotated-gradient curvature residual
to the *ownership margin*, with zero response through the L-1 pair altitudes.
Do not change `Sample::sea`, elevation, the categorical biome comparator, or
water geometry to solve a color-contour defect.

Andrew knobs if the fallback is needed:

- `COAST_RANGE_CURVE_START_KM`
- `COAST_RANGE_CURVE_FULL_KM`
- `COAST_RANGE_CURVE_WAVELENGTH_KM`
- `COAST_RANGE_CURVE_AMPLITUDE_KM`

### B-18: explicit conserved-mean payload

- Add one compact UNORM8 RGB mean to the terrain vertex payload. The value is
  the CPU's existing `target_mean`; it is presentation data, not a new world
  field.
- Pack it into the existing interstage-location budget by changing the scalar
  distance varying into `vec4(distance, mean.rgb)`.
- In the fragment:
  1. keep the current categorical endpoints, thresholds, comparator, AA,
     beach owner, and L-1 contrast;
  2. retain the old reconstructed mean only as the categorical field's local
     centering measurement;
  3. recenter the categorical deviation around the explicit mean:
     `explicit_mean + contrast * (categorical - reconstructed_mean)`.

This makes the expected color a direct linear carrier. It removes the
endpoint×threshold cross-term while preserving the approved categorical
silhouette and contrast. At the 4–8 km handoff the explicit mean is the same
CPU mean used to model the exact local/voxel mixture, so descent converges
without a tone ramp.

Andrew knob remains:

- `BIOME_RANGE_CATEGORICAL_CONTRAST` (pattern contrast only; it must no
  longer change the mean).

## Invariants and tests

- No shared categorical class/ownership field change is planned.
- `Sample::sea`, water geometry/physics, biome warp, comparator seeds,
  thresholds, and voxel material decisions remain untouched.
- Add a dense predicate test proving the new signed sea margin has exactly
  the same sign as `sea_from_fields`, including both OR branches and equality
  edges.
- Add a payload/interpolation test proving an explicit mean remains affine
  between vertices while the old endpoint×threshold reconstruction does not;
  categorical recentering must preserve that explicit expectation.
- Compare all required before/after captures, repeat selected captures for
  byte determinism, then run release-lib tests, verify, sync-diff, sky reel,
  world reel without `--accept`, hitch probes, and census discipline.

## Implementation/results log

Design frozen here before renderer changes. Results, changed reel poses,
determinism hashes, gate output, final knobs, sheets, and clip paths will be
appended as work proceeds.

### Iterations 1-2: falsified hypotheses

- The continuous sea classifier margin is sign-exact (focused test passes),
  but by itself does not remove the selected orbital macro-steps. It restores
  information to the signed contour; coarse sea vertices still contribute
  water RGB as their ground backing, so the reconstructed contour cannot
  reveal curved dry-side color through most mixed triangles.
- A first range-only, rotated-gradient shoreline residual changes pixels only
  near the selected coasts, but the water-color backing still dominates and
  the long orthogonal facets remain. The next B-17 change must give mixed sea
  triangles honest land backing before tuning the residual.
- Carrying the explicit CPU range mean changes `f000035` by at most one byte.
  The proposed endpoint-times-threshold interpolation cross-term is therefore
  real algebraically but not the reported seam's material cause. Do not retain
  four extra vertex bytes for a visually irrelevant mechanism.

### B-18 revised diagnosis from carrier isolation

`TRI_LOD_RING_DEBUG` proves that f000035 contains only mesh levels 13
(roughly 66 m vertex spacing) and 14 (roughly 33 m). Their polygonal boundary
cuts diagonally across the broad ecotone; it does not coincide with the
near-vertical world-space biome gradient.

Unlit probes of the local mesh tint, reconstructed mean, thresholds, and
normal remain continuous at that exact L13/L14 boundary. After a one-pixel
blur, the measured signed RGB residual across 3,131 boundary samples is below
0.1 byte/channel in the production frame. Rendering the same pose with
`TRI_NO_IMPOSTORS=1 TRI_NO_ROCKS=1` removes every reported dark dither mark
and leaves a smooth ground gradient. The marks persist on top of every unlit
terrain probe, proving they are separate forest-impostor geometry rather than
the categorical terrain shader.

The actual LOD-dependent carrier is `impostor_stride_for_level`:

- level 14 samples the authoritative column candidate lattice at stride 1;
- level 13 samples at stride 2 and multiplies the placement probability by
  stride squared;
- this conserves only the statistical count, not the realized placements,
  local species/color mix, canopy coverage, or convergence to stride-1 voxel
  trees;
- per-tile caps and area-conserving billboard boosts add a second
  level-dependent coverage/tone change.

Thus adjacent rings can have equal *expected tree count* while rendering
different realized tint and dither density. The descent re-rolls those
realizations until level 14/voxels restore the stride-1 field.

### B-18 revised design

Make the visible mesh/voxel forest use one canonical stride-1 ownership
stream wherever individual impostors are drawn. Any range budget reduction
must be a deterministic, world-anchored subset with coverage compensation
defined independently of tile level and tile extent. In particular:

1. levels 13 and 14 must enumerate the same stride-1 candidates;
2. survivors must be selected by a world candidate hash against a
   distance/footprint band, never by a per-tile candidate count;
3. billboard area/color compensation must depend on that same continuous
   footprint band, not on `cands.len()` or quadtree level;
4. the level-14 result must be the exact near/voxel appearance and the
   level-13 handoff must preserve the same local mean and nested survivors.

Keep a read-only LOD false-color probe and add focused stream tests before
accepting the replay. The explicit biome-mean payload will be removed unless
a later isolated terrain case demonstrates a nontrivial benefit.

### Iterations 3-6: measured fix

`TRI_IMPOSTOR_STATS` quantified the dense side of the f000035 handoff:

- representative L13 tiles: about 7,400 candidates, 1,400 retained
  (`keep_ratio` about 0.19), old billboard scale capped at 2.2;
- representative L14 tiles: about 1,800 candidates, 1,400 retained
  (`keep_ratio` about 0.75), old integer scale `sqrt(ceil(1800/1400)) =
  sqrt(2)`.

The old integer scale therefore over-covered a merely partial L14 cap by
about 50%, while the L13 side had four times fewer visible marks per unit
area. In a 12-pixel strip on each side of the exact ring, forest-impostor
energy was 2.13x larger on L14 than L13. Replacing only the integer scale with
the exact ratio improved that to 1.60x, proving both the tint and density
parts of the report.

Enumerating stride 1 everywhere or scaling the L13 cap by physical tile area
would make individual tiles 5+ MB and re-open the 512 MB/1 GiB cache
thrash/OOM class. A trial that made deep tiles uncapped immediately produced
845-937 MB settled-capture cache storms, so that route was rejected and
removed.

The accepted representation keeps the existing bounded CPU candidate
supersets and moves *visible* ownership to camera distance:

1. Each tree packs its conditional placement ticket into payload-off
   `beach.y`.
2. The otherwise-dead biome0/1 RGB bytes carry a precise root-relative
   vertex offset; biome2.r carries the CPU superset retention probability.
   Vertex size remains the approved 104 bytes.
3. One shader probability curve is below the available supersets at every
   LOD handoff (about 4.7, 9.5, 19, and 38 km). Both rings therefore show the
   same expected visible density at their shared edge.
4. The vertex shader applies the same probability-derived canopy scale on
   both rings: lateral footprint receives area compensation, height only its
   approved weak quarter-power response.
5. Probability reaches one at the near endpoint and zero before the L11/L10
   edge, continuously handing individual silhouettes to the existing
   LOD-invariant far-canopy tint.

On f000035 the same 12-pixel measurement is now 1.04x overall and 1.06x in
the forest interior (upper forest segment 0.97x). The polygonal L13/L14 edge
is no longer visible. The explicit biome-mean experiment was removed in full;
there is no vertex-size/cache tax for a falsified cause.

### B-17 reconstruction result

The continuous sea margin alone could not reveal its contour because mixed
sea vertices still supplied water RGB as their *ground*. Adding an honest
land/range backing only on the one-vertex mixed-sea ring made the signed
fragment contour authoritative, matching the already-approved lake-edge
strategy. Open-water interiors remain the cheap water backing.

The three baseline specimens changed as follows:

- `candidate_l1_north`: the literal rectangular bay notches and long
  horizontal/vertical staircase collapsed into a continuous rounded coast;
- `candidate_rec001`: the large vertical step polygon became one coherent
  irregular coastal curve;
- `candidate_inland_sea`: the polygonal/sawtooth macro rim became a
  continuously warped shoreline.

A range-only, rotated-gradient residual bends the remaining saturated
coarse-triangle chords without changing physical sea ownership. It starts
above 12 km (zero throughout the L-1 acceptance views), is fully active by
60 km, retires sub-pixel octaves at the same Nyquist shoulder as L-1, and
widens only orbital shoreline AA to 1.35 pixels. Final curve gain is 4.2 m
against the existing 5 m carrier; edge gating prevents new water in pure
land/water triangles.

## Final acceptance and gate transcript

### Required visual contracts

- The complete 112-pose recording was rendered after the final code with the
  mission command:

  ```text
  TRI_POSE_RENDER=1 cargo run --release --example play -- scripts/rec001-replay.play
  ```

  Archived result: `viewer/interchange/lodcolor/after/rec001/frames/`.
  Required frames `f000020`, `f000035`, `f000060`, `f000090`, and `f000111`
  are in the before/after replay sheet. The f000035 L13/L14 polygon no longer
  changes expected forest tint or mark density; the descent reaches the
  near/voxel frames without a tone step.
- Final B-17 specimens:
  `viewer/interchange/lodcolor/after/orbit_closeups/`.
- Final L-1 accepted-character views:
  `viewer/interchange/lodcolor/after/l1_acceptance/`.
  The two required altitude pairs retain the approved boundary silhouette,
  contrast, palette, and patch character. B-17's altitude gate is exactly
  zero in all four views.

### Automated gates

- `cargo test --release --lib`: **134 passed, 0 failed, 2 ignored**.
  The two mission additions are:
  `sea_ownership_margin_is_sign_exact_and_analog` and
  `tree_impostor_payload_roundtrips_without_enabling_biomes`.
- `bash scripts/verify.sh` was attempted literally, but this Windows host has
  no installed WSL distribution, so Bash cannot start. The script was read
  and its exact underlying build/survey loop was run directly: all eight
  suites passed (`physics-regressions`, `water-regressions`,
  `lake-regressions`, `flooded-caves`, `invariant-survey`, `auto-survey`,
  `camera-controls`, `solar-system`). The optional survey NPZ was absent, so
  the script would also have skipped regeneration.
- `sync_diff.py --label B17-B18-lodcolor`: passed; `sea_calib` is exactly
  **0.0% / 0.0 / 0 / +0.0**. Report:
  `viewer/interchange/runs/sync-diff/report.md`.
- `sky_reel.py`: **all 5 poses byte-clean**.
- `world_reel.py` without `--accept`: **all 29 poses clean**; changed-pose
  list is empty. The mission changes are deliberately outside the standing
  reel's tolerances/poses and remain explicit in the dedicated sheets.
- `git diff --check`: clean (only Git's standing LF/CRLF notices).

The shared categorical ownership field did not change. `Sample::sea`,
`sea_from_fields`, biome weights/thresholds, comparator seeds, voxel
materials, and water geometry/physics are unchanged, so the conditional
water-census-cap gate was not triggered. Sea presentation gained an
exact-sign analog margin and mixed-triangle backing only.

### Hitch and determinism probes

- `b4a-forest-descent.play`: worst teleport p95 385.03 ms; the productive
  forest descent steps are otherwise below 75 ms.
- `b4a-lateral.play`: known cold first-pose build 1797.07 ms; every subsequent
  39 km lateral step is at or below 21.53 ms.
- `b4-ascent-perf.play`: known cold first pose 763.99 ms; subsequent ascent
  steps are at or below 39.83 ms.
- Repeated f000035 captures: 0 mismatches across PNG + JSON.
- Repeated seven-pose orbital captures: 0 mismatches across 14 PNG/JSON
  files. Representative SHA-256 recorded by the probe:
  `ED773170A16FFA6EB5EAF25FC74091EE7A7CA673E16696383C46547341B5713D`.

Settled multi-teleport instruments still emit the renderer's existing strict
cache-eviction tripwire when accumulated off-screen tiles exceed the 512 MB
budget (the full replay logged frames 29/34/41). Runs completed, repeated
captures are byte-identical, vertex size remains 104 bytes, and the live
lateral probe shows no moving-flight recurrence.

## Andrew-facing knobs

Range coast:

- `COAST_MARGIN_ELEVATION_SPAN_KM`
- `COAST_MARGIN_WATER_FRACTION_SPAN`
- `COAST_MARGIN_OCEAN_FRACTION_SPAN`
- `COAST_RANGE_CURVE_START_ALT_KM`
- `COAST_RANGE_CURVE_FULL_ALT_KM`
- `COAST_RANGE_CURVE_BASE_WAVELENGTH_KM`
- `COAST_RANGE_CURVE_CARRIER_GAIN_KM`
- `COAST_RANGE_AA_WIDTH_PX`

Seamless forest handoff:

- `TREE_IMPOSTOR_FULL_DISTANCE_KM`
- the four named LOD-handoff distance/probability knots
- `TREE_IMPOSTOR_ZERO_DISTANCE_KM`
- `TREE_IMPOSTOR_MAX_SCALE`

The existing `BIOME_RANGE_CATEGORICAL_CONTRAST` remains a pattern-contrast
knob; the isolated B-18 carrier was not the categorical shader.

## Review artifacts

- Contact sheets:
  `viewer/interchange/lodcolor/contact_sheets/`
  - `01_b17_orbit_before_after.png`
  - `02_b18_replay_before_after.png`
  - `03_l1_acceptance_before_after.png`
  - `04_b18_f35_diagnostic.png`
- Feature clip:
  `viewer/interchange/lodcolor/feature_clip/lodcolor_feature.mp4`
  (8.0 s, 1280x720, 30 fps, 240 frames, H.264; SHA-256
  `48B23BBDCC00C582029AA18F56D655155D5785837B160B908C817386B581706B`).
- Repro scripts:
  `viewer/scripts/lodcolor-orbit-candidates.play`,
  `viewer/scripts/lodcolor-rec001-keyframes.play`,
  `viewer/scripts/lodcolor-rec001-f35.play`,
  `viewer/scripts/lodcolor-feature.play`, and
  `viewer/scripts/lodcolor_contact_sheets.py`.

All implementation and evidence remain uncommitted, as requested.
