# LANDSCAPE Track C — strata banding + coastal bathymetry color

Base: `cb7301e`  
Scope: fragment color only in `viewer/src/shader.wgsl`; no Track A source, vertex/impostor path, terrain data, renderer layout, or uniform plumbing changed. No commit or reel baseline was made.

## Result

Neisor now carries deterministic sedimentary color beds on steep exposed rock/earth in both mesh and voxel fragments. The approved material color remains the base: strata multiply it by a restrained hashed mineral palette instead of replacing it. Flat terrain, grass, snow/ice, foliage, distant/unresolved beds, and the Moon are excluded.

True shallow sea now grades toward turquoise while deep sea retains the existing blue. The fragment stage has no scalar depth or ocean-class varying, so this pass decodes the existing true-ocean 0–20 m shoal signal already embedded by `terrain::water_surface_color` in interpolated water RGB. Fresh rivers/lakes and the deep-ocean control remain effectively byte-clean.

## Strata field

For fragment-relative position `rel`, planet center `center`, original material color `c`, and normal `n`:

1. `surface_dir = normalize(rel - center)` supplies a world-anchored domain.
2. Steepness is `1 - abs(dot(normalize(n), surface_dir))`, faded from 0.08 to 0.28. A cheap camera-up test rejects flat fragments before the expensive work.
3. The material gate classifies neutral rock/stone/gravel and ordered warm earth from ratios in the original approved albedo. Value/chroma limits reject trunks, grass/leaves, snow, and ice.
4. Camera-relative radial expansion reconstructs precise local elevation without taking a planet-magnitude `length`:

   `h_m = 1000 * (eye_elevation + radial + (|rel|² - radial²) / (2 * eye_radius))`

5. Regional folding is:

   `folded_m = h_m + 260 * kgnoise(96 * surface_dir, karst.w + 0xDED7DCEC)`

   Frequency 96 gives an approximately 175 km wavelength. The seed delta maps the established premixed `karst.w` stream to the independent +85021 stream; no raw seed is remixed in the shader.
6. `bed = folded_m / (9 m * thickness_scale)`. `floor(bed)` hashes into five multiplier colors: cool shale, pale ochre, umber, warm sandstone, and pale mineral. `fwidth(bed)` anti-aliases transitions and retires the field when a pixel spans more than one bed.
7. Exposure also fades from 25–80 km. The final value is `mix(1, palette_multiplier, exposure * 0.28)`, preserving the existing material response and limiting the strongest default channel movement to roughly 8%.

This helper is called once in `fs_main` after the existing range-ground reconstruction and before weather/lighting. Mesh and voxel fragments therefore share one truth. `fs_moon` is a separate entry point and is untouched.

## Knobs and defaults

| Knob | Default | Meaning |
|---|---:|---|
| `STRATA_BAND_STRENGTH` | `0.28` | Blend from the approved material to the multiplier palette |
| `STRATA_BAND_THICKNESS_SCALE` | `1.0` | Scale on the 9 m reference bed thickness |
| `STRATA_TILT_AMPLITUDE_M` | `260.0` | Large-scale vertical warp/fold displacement in metres |
| `BATHYMETRY_SHALLOW_TINT` | `(0.035, 0.42, 0.36)` | Turquoise shallow-sea endpoint |
| `BATHYMETRY_DEPTH_FALLOFF_SCALE` | `1.0` | Scale applied to the recovered 0–20 m shoal carrier |

Supporting fixed defaults are 9 m reference beds, fold frequency 96, five multiplier swatches, and 0.55 maximum shallow-tint mix.

## Bathymetry signal and preservation

The shared CPU water ramp's fresh/deep portion is affine:

`green_base = 0.6408163 * blue - 0.0222449`

Only a true-ocean sample in the existing first 20 m shoal ramp rises above that line. The fragment recovers:

`carrier = clamp((green - green_base) / 0.082551, 0, 1)`

and uses it as the plausible depth proxy. Vertex interpolation preserves the zero residual for fresh water because every endpoint lies on the same affine line. A red-channel rejection excludes the existing mineral-pale salt/frozen family. The same decoder serves mesh water and block liquid.

Preserved contracts:

- The `4f47b6c` true-ocean-mask lesson remains upstream: only CPU samples classified as true sea receive the carrier. A below-sea river mouth therefore does not become turquoise.
- Rivers and lakes remain outside the effect; the fixed fresh-water control has 0.00% of pixels above a 2-level delta.
- Deep open ocean has 0.00% above a 2-level delta (maximum one 8-bit quantization tick).
- Ice itself is not tinted. `sea_ice_summer` changes only in exposed true-sea water between summer ice.
- Underwater-view tinting still runs later in the original path and was not edited.
- W2 water/storm-edge lighting and every sky path were not edited.

## Visual evidence

Fixed before/after capture deltas use worst-channel 8-bit pixel differences:

| Pose | Mean delta | Pixels >2 | Pixels >6 | Max | Reading |
|---|---:|---:|---:|---:|---|
| cliff | 1.037 | 17.67% | 1.05% | 8 | Subtle beds on the exposed high face |
| canyon | 2.239 | 44.47% | 0.96% | 8 | Folded layers read across both walls |
| mountain face | 0.487 | 7.00% | 0.33% | 9 | Exposed earth bands; snow remains primary |
| coast shallow | 11.946 | 84.97% | 84.90% | 17 | Clear turquoise shelf endpoint |
| coast deep | 0.000003 | 0.00% | 0.00% | 1 | Deep endpoint preserved |
| mid-altitude relief, 3 km | 0.783 | 9.76% | 0.14% | 8 | L-1(c) relief gains structure at range |
| mid-altitude flat, 6 km | 0.000 | 0.00% | 0.00% | 0 | Flat-ground gate preserved |
| fresh-water control | 0.000003 | 0.00% | 0.00% | 1 | No sea tint |
| Moon control | 0.000 | 0.00% | 0.00% | 0 | Separate material path unaffected |

Contact sheets for Andrew (keep the folder together; the SVG sheets reference their sibling PNG evidence):

- [Final category index](viewer/interchange/strata/contact-sheet-index.svg)
- [Strata before/after with detail crops](viewer/interchange/strata/contact-sheet-strata.svg)
- [Shallow/deep/fresh-water controls](viewer/interchange/strata/contact-sheet-water-controls.svg)
- [Mid-altitude and Moon controls](viewer/interchange/strata/contact-sheet-range-controls.svg)
- [World-reel changed-pose crops](viewer/interchange/strata/contact-sheet-world-reel.svg)
- Raster scouting supplements: [strata scout](viewer/interchange/strata/strata-before-after-scout.png), [bathymetry scout](viewer/interchange/strata/bathymetry-before-after.png), [canyon scout](viewer/interchange/strata/canyon-low-scout-sheet.png), and [final location scout](viewer/interchange/strata/final-scout-sheet.png)

The reproducible capture script is `viewer/interchange/strata/evidence.play`; baseline, final run A, and final run B frames are under `evidence/before`, `evidence/after-a`, and `evidence/after-b`.

## Gates

| Gate | Result |
|---|---|
| `cargo test --release --lib` | PASS — 109 passed, 0 failed, 1 ignored |
| `cargo check --release --features multiplayer` | PASS — shader compiled; only three existing warnings |
| Regression play suites | PASS — physics-regressions, water-regressions, lake-regressions, flooded-caves, invariant-survey, auto-survey, camera-controls, solar-system |
| `sync_diff` | PASS — all 15 pairs rendered; `sea_calib` divergent 0.0%, mean 0.0, p95 0, max 1 |
| World reel | Expected content diff only; all void/Moon lints pass |
| Sky reel | CLEAN — all 5 poses byte-identical, including 0.000 moon ROI delta |
| Double-run determinism | PASS — all 9 fixed evidence captures byte-identical between final runs A/B |
| `git diff --check` | PASS |

The `verify.sh` wrapper could not launch because this Windows host maps `bash` to WSL and has no WSL distribution installed. Its eight checked-in `.play` suites were invoked directly with the same release harness and all passed. Managed workspace policy also prevented the Python reel/sync wrappers from writing their generated tracked play files, heatmaps, and JSON reports; the checked-in play files were rendered directly and the scripts' exact read-only lint/diff formulas were applied to the resulting frames and accepted baselines.

### World reel changes and crops

Only two of 28 poses cross the accepted-reel thresholds:

- `orbital_coast`: mean delta `2.872`, fraction >6 `18.63%`, max `100`. Expected: the true-sea shelf now reads turquoise against deep water.
- `sea_ice_summer`: mean delta `0.846`, fraction >6 `1.25%`, max `9`. Expected: exposed true-sea water between the summer ice receives the shallow tint; the ice material is unchanged.

Every other pose remains below threshold. `moon_orbit_face` and `moon_low_flyby` are byte-identical. See the linked world-reel crop sheet above; no baseline was re-blessed.

### GPU cost

`TRI_GPU_TIMERS=1`, the exact three `gputimers-smoke.play` poses, extended to 120 steady-state frames, two baseline/final A/B samples per Neisor pose:

| Terrain group pose | Baseline samples | Final samples | Mean delta |
|---|---:|---:|---:|
| Forest ground | 17.23, 17.22 ms | 17.23, 17.31 ms | +0.045 ms (+0.26%) |
| 250 km orbit | 11.96, 11.97 ms | 12.22, 12.17 ms | +0.230 ms (+1.92%) |
| Moon control | 0.11 ms | 0.11 ms | 0.000 ms |

The fragment addition stays small in the steady-state terrain budget. Flat, distant, non-earth, and unresolved strata candidates return before the fold-noise evaluation.

### Determinism proof

All nine final A/B PNGs match byte-for-byte. The two requested representative hashes are:

- `canyon.png`: `a169b50a69546be299aac9d1c5949dd29fdc76b2a60e8b876a0508598760394c`
- `coast_shallow.png`: `61ddea417b889ae5d498df8d58b8a4958dfc75fd8a1ae56a8cd8356ec299288d`

## Honest cut-lines

- There is no true scalar depth or sea bit in `fs_main`. This pass can only reshape the already-correct, true-sea 0–20 m RGB carrier; it cannot extend turquoise across a broader continental shelf. Doing so requires a depth/class payload change owned outside this fragment-only track.
- Material family is inferred from the original albedo because the fragment contract has no material ID. It is stable for the current palette and backed by grass/snow/Moon controls, but a future wholesale material-palette change should retune the ratio thresholds or add an explicit family varying.
- The 25–80 km distance fade and derivative retirement are intentional. This partially addresses L-1(c) on medium-altitude exposed relief; literal flat ground remains unbanded as required, and strata do not attempt an orbital texture layer.
- The water proxy is vertex-interpolated color, not a physical per-pixel ray depth. It is deterministic and classification-safe, but should not be interpreted as metres beyond the existing carrier.
- CPU census/column census data were not regenerated because this diff changes no terrain, hydrology, vertex classification, or serialized data. The water and invariant render suites, sync-diff, reels, and direct visual controls cover the shader coupling.
- No renderer uniform plumbing was necessary. The requested art knobs are named shader constants, keeping the merge surface to `shader.wgsl` plus this findings document and ignored interchange evidence.
