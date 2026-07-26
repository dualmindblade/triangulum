# Rivers iteration 3 findings

Mission: close BUGS.md B-13, its coarse-LOD sawtooth channel edge, B-14,
and B-15 without changing `rivers.bin`.  Worktree branch is
`tri-sol-rivers3` at `71265e7`; no commits will be made.

## Design (written before code, 2026-07-18)

### Shared constraints

- `terrain::sample` / `RiverIndex::river_near` remain the world authority.
  No camera state, wall clock, mutable history, or new baked data enters the
  field.  Renderer-only footprint filtering may change presentation, never
  water/land class, water level, collision, or the displaced river course.
- Mesh and voxels continue to consume the same fall/rapid and shore values.
  A dry fragment selected by the signed shore field must reveal the land
  material behind the water, not a separately interpolated water-colored
  mesh base.
- Existing unrelated dirty files (`viewer/assets/weather_tuning.json`, edit
  journals, logs, and `MISSION.md`) are out of scope and will not be touched.

### B-13: prominent falls, restrained rapids

The current loader evaluates every 30--40 km drainage segment independently
with `fall_strength`, then immediately uses every passing segment both to
concentrate the hydraulic profile and to emit sheet/foam art.  The baked-data
census has 20,504 segments and 18 raw candidates.  The approved
`9.79546 107.60747` site is segment 8372 (34.279 km long, 326.1 m baked drop,
strength 0.409).  Its same-reach neighbor is 66.2 km of arc away and weaker
(segment 8423, strength 0.147).  A second long reach contains candidate
clusters at 64--71 km spacing.  Thus the diagnosis is candidate density on a
continuous steep reach; no `rivers.bin` change is needed.

After `build_reaches`, retain the raw score long enough to run a deterministic
promotion pass per junction-to-junction reach:

1. A true fall must be the strongest candidate in a
   `FALL_PROMINENCE_RADIUS_KM = 72` arc neighborhood.  Equal scores use baked
   drop, then stable segment id, so the result is total and deterministic.
2. Surviving local maxima are considered strongest-first and must be at least
   `FALL_MIN_ARC_SPACING_KM = 96` apart.  The raw score is copied back
   bit-for-bit only for promoted sites, preserving the approved site's
   concentrated level profile, foam, sheet geometry, and close-range look.
3. Passing candidates that are not promoted keep a separate `rapid_strength`.
   It never changes water level.  A flow-scaled, endpoint-tapered, narrow
   channel-core mask applies at most a weak fraction of the existing foam
   color on both mesh and voxel water.  This reads as continuous rapids rather
   than another falling sheet.

Andrew-facing additions stay in `river_tuning`: prominence radius, minimum
spacing, rapid maximum foam strength, rapid flow shoulder, endpoint ramp, and
core-width shoulder.

### Coarse-LOD sawtooth channel edge

The displaced centerline itself is correct and must not be attenuated or
phase-shifted by LOD.  The artifact comes from presentation ownership:
`tile_wet` already hands an under-resolved river to a widened,
coverage-correct soft carrier, but `shore_field` then unconditionally restores
a crisp interpolated zero contour.  On kilometre-scale triangles that contour
connects sparse meander samples as hard zigzags.

Keep the world-space centerline and zero crossing unchanged.  Reuse the
existing half-width/vertex-spacing shoulder (0.9--2.0) as a river-only
fragment-field gain.  Encode that gain in the otherwise-unused negative mesh
`wflag` range.  Under-resolved reaches use the existing soft, coverage-correct
paint; as the bank becomes resolvable the exact signed contour gains opacity
at the same location.  Lakes, seas, ponds, voxel faces, and positive
fall-sheet flags retain their current paths.  This is footprint-aware bank AA,
not LOD-dependent meander geometry, so descending cannot make the channel
swim sideways.

### B-14: lake-beach footprint retirement

The lake-shore sand mix is currently baked into every land vertex color and
every range-biome endpoint.  Consequently the 1.5 m height carrier remains a
pale categorical rim even when it is below a pixel, and the shader has no
separate signal with which to retire it.

Carry lake-shore coverage through the existing beach payload, with a distinct
lake mode (the generic-beach fine-gain byte is zero; generic coast values are
at least 0.08).  Local/arrival color keeps the approved shared
`lake_shore_frac`; range endpoints represent the underlying ground and apply
the sand owner in the fragment.  For a lake shore, estimate carrier cells per
pixel from derivatives of the signed shore field divided by the physical
1.5 m height band.  Apply the same 0.25--0.50 cells/pixel Nyquist shoulder used
by L-1: resolved beach stays sand, while a sub-pixel band continuously retires
to its underlying ground instead of surviving as an outline.  The generic
beach comparator's 7 km / 440 m octaves will also use the same derivative
shoulder instead of the old camera-distance-only fine-detail fade.

### B-15: mesh water must reveal the shore authority

At present a liquid-lake mesh vertex sets both `color` and `water.rgb` to
water.  The fragment shore field can correctly choose the dry half of a mixed
triangle, yet `wet = 0` merely reveals an interpolated blue "ground" base.
That is the observed ghost water arc over voxel sand.

Liquid lake vertices will carry an ordinary land/biome backing payload (sand
backing near the baked lake edge) while `water.rgb` remains water and the
signed shore field remains the sole wet coverage authority.  Open-water
pixels are unchanged because their coverage is one; dry pixels in a mixed
shore triangle reveal the same lake-beach material family as voxels.  Frozen
lakes and the sea carrier keep their established paths.  A regression at
Austin's `17.433 28.979` shore will assert that voxel-class dry samples cannot
receive mesh water ownership or a water-colored backing.

## Implementation and closure

No baked data or hydraulic class changed.  `viewer/assets/rivers.bin` is
untouched; all new state is derived deterministically while the existing RIV1
stream loads, or is a fragment-footprint presentation value derived from the
shared shore field.

### B-13: one prominent fall, demoted rapid run

- `RiverIndex::load` now runs `apply_fall_prominence` after assigning stable
  reach ids and arc distances.  Raw candidates are compared only within their
  junction-to-junction reach.  The pass has total tie-breaking (strength,
  baked drop, then segment id), and its strongest-first spacing pass is also
  stable.
- The production bake contains 20,504 segments and only 18 raw fall
  candidates.  Approved segment 8372 retains its original `fall_strength`
  bits (`0.409` at the photographed site); segment 8423, 66.2 km away on the
  same steep reach, loses its concentrated fall profile and becomes a rapid.
- A demoted site's level returns to the original linear hydraulic profile.
  `rapid_strength` contributes only weak, flow-scaled, endpoint-tapered,
  center-channel aeration.  Mesh and voxel water use the same
  `max(fall foam, rapid foam)` value and color, so changing renderer or LOD
  cannot change the feature's location or hydraulic truth.
- The old 3 km view's long white staircase is gone.  The retained fall is a
  compact white mark on a narrow blue reach; the continuous steep run reads
  as restrained narrow aeration rather than more falling sheets.  The close
  fall sheet, plunge, silhouette, and level profile remain the approved art.

### Coarse river edge: footprint carrier without centerline motion

- River mesh vertices encode signed-field ownership in the unused negative
  `wflag` range: `-1` is coverage-correct paint only and `-2` is full signed
  shore ownership.  The shader fades the exact field in between; its zero
  remains the same displaced centerline at every level.
- The final measured shoulder is 2--4 channel half-widths per vertex spacing.
  Below it, the existing carrier widens by `1.60 * spacing` and reduces
  opacity by covered width.  This retires the hard kilometre-vertex chords
  while preserving the integrated blue thread.
- `wet_soft` was the remaining bypass: it contains legacy river and pond
  coverage and its `max` restored the old hard, broad river after the new
  carrier ran.  It is now consumed only when a real pond level exists.
  Rivers exclusively use the footprint-aware lane; ponds retain their old
  behavior.  A regression test locks this separation.

### B-14: lake-beach Nyquist retirement

- Lake shore sand now has an explicit beach payload mode (`beach.y == 0`).
  Local ground still uses the shared `lake_shore_frac`; range biome endpoints
  describe the underlying ground, with the fragment shader applying the sand
  owner.
- The shader divides the signed-shore derivative by the physical 1.5 m beach
  carrier and applies the same 0.25--0.50 cells/pixel Nyquist shoulder as the
  L-1 border pass.  The generic beach comparator's 7 km and 440 m carriers now
  use that footprint shoulder too.  At Austin's 24.060 km pose the crisp pale
  outlines retire continuously, without changing a lake boundary.

### B-15: signed shore is the mesh/voxel boundary

- A liquid-lake shoal that voxel quantization makes walkable is now negative
  in the signed shore field.  Wet and dry sides therefore agree with the
  authoritative block predicate, including equal-block shoals.
- A mixed lake shoreline's water vertices carry honest land/biome backing
  while `water.rgb` remains water.  The exact signed field reveals that
  backing on dry fragments instead of revealing an interpolated blue base.
  The full payload is limited to the one-vertex mixed-shore ring; open-water
  interiors retain the cheap water carrier.
- The B-15 test walks an 81 x 81 sample window around
  `17.433 28.979`, requiring wet samples to have positive shore ownership,
  voxel-dry lake material to have non-positive ownership, and wet edge backing
  to differ materially from water.  The photo now shows the formerly blue
  shoal as dry connected land, matching the voxel/finer-mesh footprint.

## Andrew tuning handoff

All river values below live in `rivers::river_tuning`; the lake footprint
values are shader/shared-shore constants.

| Dial | Final | Effect |
|---|---:|---|
| `FALL_PROMINENCE_RADIUS_KM` | 72 km | Local-maximum arc neighborhood within one reach. |
| `FALL_MIN_ARC_SPACING_KM` | 96 km | Minimum arc distance between promoted fall sheets. |
| `FALL_FOAM_UPSTREAM_REACH_KM` | 0.55 km | Compact upstream foam around a true fall; downstream remains 0.90 km. |
| `RAPIDS_MAX_FOAM_STRENGTH` | 0.18 | Hard ceiling for demoted-candidate aeration. |
| `RAPIDS_FLOW_START_M3S` / `FULL` | 300 / 1,800 m3/s | Flow shoulder for rapid visibility. |
| `RAPIDS_ENDPOINT_RAMP_KM` | 1.2 km | Removes abrupt rapid color starts at segment ends. |
| `RAPIDS_CORE_FULL_HALF_WIDTH` / `END` | 0.18 / 0.62 | Narrow channel-core shoulder in half-width units. |
| `LOD_SHORE_FIELD_START_HALF_WIDTHS` / `FULL` | 2.0 / 4.0 | Exact river-bank field ownership shoulder. |
| `LOD_SHORE_PAINT_FEATHER_SPACING` | 1.60 | Under-resolved carrier width in vertex-spacing units. |
| `LAKE_BEACH_HEIGHT_CARRIER_KM` | 0.0015 km | Physical lake-beach carrier used for footprint estimation. |
| border Nyquist `FULL` / `CUTOFF` | 0.25 / 0.50 cells/px | Shared lake/generic beach retirement shoulder. |

Prominence and rapid dials do not alter water level, width, collision, or
course.  The LOD dials alter only ownership/coverage around the same signed
zero.  B-15 intentionally has no art dial: block-quantized shore class is the
authority.

## Before/after evidence

All comparisons pin seed, season, absolute weather time, weather cover, and
sun.  `fraction > 6` is the portion of pixels whose largest 8-bit channel
change exceeds six; it reports the size of the intended edit, not a quality
score.

| Repro | Result | Mean max-channel delta | Fraction > 6 |
|---|---|---:|---:|
| B-13 fall, 3 km | Cascade chain and broad hard zigzag retire; one compact fall remains on the unchanged course. | 1.529 | 4.814% |
| B-13 fall, 1.5 km | Chained sheet becomes a narrow rapid/fall carrier. | 1.093 | 2.386% |
| approved fall, 80 m | Approved sheet and plunge silhouette preserved. | 0.365 | 0.545% |
| B-14, 24.060 km | Pale sub-pixel lake outlines retire. | 0.529 | 2.235% |
| B-15, 1.012 km | Blue mesh overlap becomes dry shore-connected land. | 2.970 | 5.519% |

The B-13 plain control at 400 m is byte-identical before/after.  Its 1.5 km
frame changes only where the deliberately revised coarse river carrier is
visible.  The close fall's small numerical delta is upstream water/foam
extent; side-by-side inspection confirms that the true sheet geometry and
plunge art are unchanged.

Final repro inputs:

- `viewer/scripts/b13-repro.play`: fall at 3 km, 1.5 km, 80 m, and approach;
  plain/control site at 1.5 km and 400 m.
- `viewer/scripts/rivers3-photo-repro.play`: Austin's exact B-14 and B-15
  coordinates.
- `viewer/scripts/rivers3-hitch.play`: paired 20-frame cold and 30-frame warm
  probes at all five mission distances.
- `viewer/scripts/rivers3-glide.play`: continuous approved-fall approach and
  pull-back through 1.5 km to 3 km.

## Gate ledger

| Gate | Final result |
|---|---|
| `cargo test --release --lib` | PASS: 132 passed, 0 failed, 2 ignored.  Baseline was 128 passed + 2 ignored; four passing mission regressions were added. |
| `bash scripts/verify.sh` | PASS: all eight suites green (`physics-regressions`, `water-regressions`, `lake-regressions`, `flooded-caves`, `invariant-survey`, `auto-survey`, `camera-controls`, `solar-system`).  On this Windows host plain `bash` is the WSL stub with no distro, so the unchanged script was run through Git Bash. |
| `sync_diff sea_calib` | PASS: `0.0%`, mean/p95/max delta all zero; report label `rivers3-final`. |
| water census `--caps export` | PASS on the final binary: 104,761,853 samples in 27 s; 53,325 raw discontinuity findings grouped into 15,003 diagnostic sites; liquid lake-wall caps file is exactly `[]`. |
| `sky_reel` | PASS: all 5 poses clean; an immediate rerender changed 0/5 hashes. |
| `world_reel` without `--accept` | PASS: all 29 poses lint-clean; `--accept` was not used.  `waterfall_site` shows the range staircase gone. |
| in-class hitch probes | PASS for settled/warm captures; table below. |
| repeated-capture determinism | PASS: all 16 required PNG/JSON repro artifacts are byte-identical to an immediate second capture.  Runtime logs differ only in measured timing. |
| before/after repro and contact sheets | PASS: all required coordinates and both B-13 altitudes captured; seven final sheets listed below. |
| feature clip | PASS: H.264/yuv420p, 1280 x 720, 30 fps, 270 frames, 9.000 s, 5,971,606 bytes. |

### Reel accounting

The fresh worktree does not contain `interchange/reel/accepted` or
`interchange/skyreel/accepted`, so an historical accepted-baseline diff is
not available locally.  This was not repaired with `--accept`.  The reel
scripts themselves report clean, sky was verified by an immediate byte
repeat, and world was compared against the preserved pre-final-carrier
current reel.

Exactly nine world poses changed bytewise in that final comparison:

- `dossier_band`
- `dossier_shore`
- `jungle` (only a max-channel delta of 3)
- `mid_altitude`
- `orbital_coast`
- `river_valley`
- `river_wide`
- `taiga` (only a max-channel delta of 1)
- `waterfall_site`

The material changes are expected at river, lake-shore, and orbital/coarse
water views.  The two named tiny changes are below the sync-diff noise floor.
`waterfall_site` has the largest intended footprint (5.227% of pixels over
the noise floor), and visually demonstrates the B-13 closure.

### Hitch probes

`p95` deliberately includes the first frame in each 20-frame cold probe, then
reports a settled 30-frame run at the same pose.

| Pose | Cold p95 | Warm p95 |
|---|---:|---:|
| B-13 approved fall, 80 m | 1097.13 ms | 35.25 ms |
| B-13, 1.5 km | 24.83 ms | 12.51 ms |
| B-13, 3 km | 12.37 ms | 12.06 ms |
| B-15 shore, 1.012 km | 3971.99 ms | 36.85 ms |
| B-14 lake, 24.060 km | 30.92 ms | 22.91 ms |

The multi-second cold B-15 teleport is location streaming/tile construction,
not the mixed-shore backing payload: a diagnostic build with that backing
disabled still measured 3925.76 ms cold at the same pose.  Warm behavior is
in class.  The close-site cold cost likewise settles to the established
voxel-site range.  No assertion, device, or capture stall occurred.

## Deliverable paths

- Summary sheet:
  `viewer/interchange/rivers3/contact-sheets/00_rivers3_summary.png`
- B-13 sheets:
  `01_b13_fall_3km_before-left_after-right.png`,
  `02_b13_fall_1500m_before-left_after-right.png`, and
  `03_b13_close_art_before-left_after-right.png`
- Lake sheets:
  `04_b14_24km_before-left_after-right.png` and
  `05_b15_1km_before-left_after-right.png`
- Feature milestones:
  `06_feature_glide_milestones.png`
- Before/after frames:
  `viewer/interchange/rivers3/before/` and
  `viewer/interchange/rivers3/after/`
- Determinism repeat:
  `viewer/interchange/rivers3/determinism-repeat/`
- Final hitch log:
  `viewer/interchange/rivers3/hitch-final-aa/run.log`
- Census report/export:
  `viewer/interchange/rivers3/census.md` and
  `viewer/interchange/rivers3/census-caps.json`
- Feature clip:
  `viewer/interchange/rivers3/glide.mp4`
  (SHA-256 `5d13fcee80cdfc2b820737f567c6b914ff8927e23ff51cb9b894e4eeb3516d27`)

No commit was made.  Existing unrelated dirty weather/edit-journal/log files
remain untouched.
