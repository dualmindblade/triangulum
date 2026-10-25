# Moonscape round 2 findings

Baseline: `main` at `51810124c3551f946dba7d6ef0649949ff54bd6d`
(`5181012 Merge branch 'sol/moon-lattice'`). The baseline benchmarks and visual
captures were made before any source edit. This worktree remains uncommitted.

## Outcome

The four requested generation refinements are implemented in the shared lunar
surface law:

1. **Impact floors level inherited relief.** Each crater owns a stable local
   excavation datum composed from terrain in cached octaves coarser than the
   impact. The visible cohorts probe the center plus eight inner-bowl bearings
   and use the local low value; fine saturation cohorts use the composed center.
   The bowl blends toward that spatially constant datum instead of subtracting
   equal depth from every inherited sample. A conservative smooth minimum with
   the pointwise inherited height is the no-raise/pillar guard. It is smooth,
   so the guard does not stamp a contour where the two candidate surfaces meet.
   Material inheritance remains impact-center owned and independent of the
   geometry datum.

   The deterministic seed-42 rim-straddling case at
   `-38.7869, -44.0389` (young radius `0.3033 deg`) changes inner-floor relief
   from `0.00432260` to `0.00003187` moon-radius units: **99.26% of the inherited
   contrast is removed**. The target never exceeds either the local excavation
   surface or the already-composed terrain, so the result cannot be the stale
   center-height pillar described in the mission.

2. **Placement no longer advertises its lattice.** `CRATER_JITTER` is raised
   from `0.14` to `0.47`, nearly the complete `+/-0.5` cell interval. A seed-42
   octave-6 face sample reaches `jitter < -0.44` and `> +0.44`. The bounded 3x3
   neighborhood remains sound: the worst elongated rim reach is
   `0.46/2 * 1.78 * 1.20 * 1.08 = 0.530` cells; jitter plus reach is `1.000 < 1.5`,
   and the feature extends `0.500` cells through an edge. The pruning margin is
   `0.54`, including more than `0.03` cell safety. The B-3 cut-line unit probe,
   cube-face/lattice continuity test, and `rimtransect.play` all pass.

3. **Sizes and impact angles are continuous.** Interior crater octave bands
   now draw log-uniformly from `0.20..0.46` cell fractions. This width is greater
   than 2x, so neighboring half-scale octaves overlap. Octave 0 retains the old
   `0.38` upper endpoint and octave 9 retains the old `0.22` lower endpoint,
   preserving the shipped absolute size bounds. Occupancy is rebalanced from
   `0.34` to `0.31` to offset the wider expected covered area while retaining
   the approximately 4x-per-octave `N(D) ~ D^-2` law. The fixed face contains
   107,990 sampled craters; its adjacent populated log bins are nonempty and
   stay within the power-law tolerance.

   Crater elongation is no longer a 2.2% binary switch. Every ordinary crater
   and ray carrier gets a continuous `1.0..1.20` draw biased by `u^7`, keeping
   strongly oblique impacts rare. The cached records contain more than 100
   distinct elongations and reach above `1.15`. Mare size families retain the
   global `1.7..17 deg` bounds but now overlap: mid `1.7..8.5 deg`, large
   `5.5..17 deg`.

4. **Maria are darker, tighter, and bolder.** Mare interiors now converge
   98.5% toward a basalt albedo target (`0.315..0.335` large,
   `0.325..0.347` mid) instead of subtracting darkness while retaining all
   highland grain. The largest seed-42 mare measures only
   `0.323518..0.324235` albedo across its inner sample, a range of `0.000717`.
   Overlaps still saturate rather than stack darkness. Low-frequency silhouette
   noise rises from `0.14 @ scale 7` to `0.24 @ scale 4.5`, with the low-order
   bearing term also increased from `0.060` to `0.070`. Its conservative reach
   is expanded from `1.48` to `1.70` radii. The deliberate tight albedo band
   (`0.965..1.075`) and wide height feather (`0.74..1.10`) are unchanged.

The 54-cell ray-carrier cohort is now immutable-cached with the coarse crater
records. This avoids paying the new datum work on the hot sample path and helps
retain the lattice branch's performance win.

## Automated gates

| Gate | Final result |
|---|---|
| `cargo test --release --lib` | **PASS** - 79 passed, 0 failed, 1 pre-existing ignored |
| Radius smoothness | **PASS** - face-0 adjacent log-bin counts `[8052, 4475, 1957, 1166, 526, 281, 121, 84, 23, 19]`; every ratio within `1.15..3.8` toward smaller radii |
| 100 random transects | **PASS** - maximum `|dh| / angular_step = 1.002716`, bound `1.10` |
| Reach/cut lines | **PASS** - analytic margin assertions plus both pruning cut lines on every crater octave |
| Lattice/cube seams | **PASS** - scalar continuity assertions |
| Batch path | **PASS** - prefetched sampling bit-identical to scalar height, albedo, smoothness, and rays |
| `rimtransect.play` | **PASS** - all 48 probes; maximum adjacent height change is about `0.84 m`, with no B-3 cut cliff |
| `moon-landing.play` | **PASS** - Neisor control, maria landing/jump/edit, moved crater floor, ray material |
| `blackhole.play` | **PASS** - B-2 height `-0.179024 km`, inside `-2.5..2.5 km` |
| `solar-system.play` | **PASS** after the two intentional lunar height-pin updates below; all ephemeris pins unchanged |
| `seasonal-cycle.play` | **PASS** - winter walk, summer swim, autumn tint, edit override |
| `moon-nav-bench.play` | **PASS** - final comparison below |
| `moon-steady-bench.play` | **PASS** - final comparison below |
| `moon-determinism.play` twice | **PASS** - both PNG sets SHA-256 byte-identical |
| `world_reel.py` | Expected two lunar findings only; all 26 Neisor poses byte-identical |
| `sky_reel.py` | Expected moon ROI finding only; all four non-moon sky poses byte-identical |
| `sync_diff.py --label moonscape-r2` | **PASS calibration** - `sea_calib 0.0% / mean 0.0 / p95 0 / bias +0.0` |

The release test's quantitative transcript also reports:

```text
mare interior albedo 0.323518..0.324235
rim-straddling floor lat=-38.7869 lon=-44.0389 radius=0.3033deg
  relief before=0.00432260 after=0.00003187
100-transect max slope 1.002716 at (14, 34, ...)
```

## Performance

Baseline navigation values are the mean of two untouched-main runs. Final uses
the same executable shape and `TRI_RAW_CAPTURE=1`. Movement rows include the
intentional asynchronous streaming spike. Negative deltas are faster.

| 30-frame hold | Main `5181012` avg ms | Final avg ms | Delta |
|---|---:|---:|---:|
| maria warmed stationary | 11.96 | 11.65 | -2.6% |
| maria movement 1 | 23.20 | 23.16 | -0.2% |
| maria movement 2 | 13.36 | 13.41 | +0.4% |
| highland warmed stationary | 13.80 | 13.68 | -0.8% |
| highland movement 1 | 15.05 | 8.61 | -42.8% |
| highland movement 2 | 11.31 | 9.67 | -14.5% |
| landed warmed steady state | 11.75 | 12.34 | **+5.0%** |

The steady p95 is essentially unchanged (`12.96 -> 12.97 ms`). The worst
slower comparison is +5.0%, within the 10% gate.

The old highland benchmark coordinate moved by kilometres under the requested
placement/floor laws and changed the visible set from 2,133 to 3,168 draws,
making its first final measurement 20.56 ms. The deliberate replacement pin
`0, 120, 8 km` renders 2,104 draws, matching main's workload rather than gaming
the comparison. Baseline logs are under
`viewer/interchange/runs/r2-baseline-nav*` and
`viewer/interchange/runs/r2-baseline-steady`; final logs are under
`viewer/interchange/runs/r2-final-nav` and
`viewer/interchange/runs/r2-final-steady`.

## Deliberate pin changes

No accepted reel baseline was modified.

| File | Old | New | Reason |
|---|---|---|---|
| `viewer/scripts/moon-nav-bench.play` highland pose | `37.0385 101.1598 2` | `0 120 8` | Requested terrain movement changed the old visible workload from 2,133 to 3,168 draws; new pin is a highland field with 2,104 draws |
| `viewer/scripts/solar-system.play`, probe `0,0` | `0.485399 km` | `-0.155138 km` | Intended randomized craters/continuous radii/floor composition |
| `viewer/scripts/solar-system.play`, probe `37,-122` | `0.054191 km` | `0.032003 km` | Same intended terrain-law movement |

`moon-landing`, `blackhole`, seasonal, orbital, gravity, physics, and material
pins required no updates.

## Reel explanations

`world_reel.py` reports exactly the lunar changes this mission is supposed to
make:

| Pose | Mean delta | Fraction > 6 | Explanation |
|---|---:|---:|---|
| `moon_orbit_face` | 7.204 | 27.50% | Full-disc mare targets/silhouettes plus randomized and size-continuous crater field |
| `moon_low_flyby` | 0.691 | 1.76% | Jittered placements, overlapping radii, continuous elongation, and leveled floors alter the local highland relief |

Every one of the 26 Neisor frames has `mean_delta=0.0`,
`changed_frac=0.0`, and a byte-identical SHA-256 hash.

`sky_reel.py` reports only `moon_zenith`: fixed 96x96 ROI mean `0.512`, fraction
`0.0207`. That tiny physical disc now shows the new generated face. The other
four sky frames (`sun_zenith`, `sunset_west`, `dusk_gradient`, `night_stars`)
are SHA-256 byte-identical. This is the intended sky-moon-only change.

## Visual evidence

The final eight-panel sheet is
[contact-sheet.png](viewer/interchange/codex/moonscape-r2/contact-sheet.png).
It contains the supplied Tycho reference, final full face, saturated highlands,
same-generation subtractive/leveled rim-straddling A/B, main/final mare
silhouettes, and the face-0 radius histogram.

Supporting artifacts:

- [crater-radius-histogram.png](viewer/interchange/codex/moonscape-r2/crater-radius-histogram.png)
- [moonfeatures.txt](viewer/interchange/codex/moonscape-r2/moonfeatures.txt) -
  107,990 radii plus deterministic feature coordinates
- `viewer/interchange/runs/r2-rim-before/rim_straddling_floor.png`
- `viewer/interchange/runs/r2-rim-after/rim_straddling_floor.png`
- `viewer/interchange/runs/r2-before/` and `r2-after/` full evidence frames

The A/B uses `TRI_MOON_LEGACY_FLOORS=1` only as an evidence lens. Both frames
therefore share all new placement, sizes, elongations, maria, camera, and light;
the only switch is subtractive versus locally leveled floor geometry. Production
rendering and every acceptance suite run with the leveled law.

## Files changed

- `viewer/src/moon.rs` - all four generation refinements, datum/ray caches,
  evidence probes, and release tests
- `viewer/examples/moonfeatures.rs` - rim-straddling probes and face histogram
- `viewer/scripts/moonscape-r2-rim.play` - exact same-generation floor A/B pose
- `viewer/scripts/moonscape_r2_contact_sheet.py` - final evidence assembler and
  histogram plotter
- `viewer/scripts/moon-nav-bench.play` - deliberate workload-matched highland pin
- `viewer/scripts/solar-system.play` - two deliberate lunar height pins
- `FINDINGS.md` - this report

No commit was created.
