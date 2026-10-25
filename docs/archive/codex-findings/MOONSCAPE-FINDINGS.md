# Moonscape v2 - Andrew crater-realism revision

Date: 2026-07-12  
Branch: `sol/moonscape` at `ab123eb`  
Result: implemented, all requested functional/performance gates green, no commit, no reel baseline accepted.

## Outcome

The old chronological fold over 378 globally stored craters is gone. Lunar terrain now uses a deterministic, scale-binned cube-sphere lattice evaluated coarse-to-fine from `(direction, seed)`. Mesh tiles, landed voxel columns, materials, and the teleport-map moon tab still consume `MoonGenerator::sample`; no shader-side lunar truth was added.

Seed 42 now has:

- 4 actual largest basins, with radii 13.33-17.31 degrees (the allowed lattice range reaches mare scale).
- Ten crater size octaves. The finest permitted radii are about 0.34-1.0 km on this 2338 km-radius moon, versus the old 2.4 km minimum.
- A fixed face-0 population probe of `[1, 2, 6, 20, 86, 348, 1456, 5500, 22425, 88678]`. From octave 2 to octave 7 the count grows `5500 / 6 = 916.7x`, close to the requested 1000x five-octave span; adjacent stable bins are approximately 4x per diameter halving, i.e. `N proportional to D^-2`.
- 10 large maria and 31 mid-size maria from a reproducible four-cluster spawn field.
- 11 sparse Tycho-size ray carriers over the whole sphere. No largest-basin octave can carry rays.

The visual gap is direct in the evidence:

- [Reference comparison and five acceptance poses](moonscape-v2/final/contact-sheet.png)
- [Teleport-map moon tab, before/after](moonscape-v2/final/teleport-map-before-after.png)
- [World-reel moon poses, before/after](moonscape-v2/final/world-reel-moon-before-after.png)
- [Neisor moon-disc ROI, before/after](moonscape-v2/final/sky-moon-zenith-before-after.png)

## Andrew's six gates and four observational notes

| Gate | Implementation / evidence |
|---|---|
| Mare-size maximum, only a few | Octave 0 has one jittered candidate per cube face, mare-field-weighted occupancy, and shallow basin depth. Seed 42 retains 4; the largest is radius 17.31 degrees (~707 km radius). |
| Much smaller minimum, ~1000x population | Ten `2^octave` grids. Radius halves and cell count grows near 4x. The fixed probe grows 916.7x over five bins and the full octave-6 population is over 1000x the 4 largest basins. |
| Saturated face | The full-face orbital bin alone contributes roughly 260 hemisphere-visible craters before the still-finer bins. The contact sheet and teleport map show hundreds, not ~30. |
| Rough tapering ray lines | Each carrier has 24 independently hashed physical-width lines: 7 long/strong and 17 shorter gap-fillers. Width decreases with radial progress; line center, length, width, breaks, warp, and strength vary independently. These are not periodic expanding angular bands. |
| Rays rare, absent on largest | A separate sparse 3x3 Tycho-size lattice produces 11 carriers on seed 42, radii 0.55-1.25 degrees. Basin octaves have no ray branch. |
| Clustered large + edge maria | Three seed-jittered near-side cluster anchors plus one seeded global cluster form the spawn field. Large candidates favor field cores; mid candidates favor the ~0.42 edge band plus sparse background. Seed 42 resolves to 10 large and 31 mid maria. |
| One-color mare-edge crater floor | Height excavates the already-composed ground, but floor albedo/smoothness are selected once at the impact center. A crater cannot sample mare on one half and highland on the other. The unit test starts opposite floor points with albedos 0.28 and 0.82 and converges them to the same inherited target. See the mare-edge panel. |
| Large rims rougher | Rim-radius angular noise falls from up to ~0.115 on basin octaves to ~0.014 on the finest bin, using two azimuthal harmonics plus seeded gradient noise. |
| Steep inside, gradual outside; large rim near ground | Inner rim width is 0.060 radius; exterior decay is 0.255 plus a falloff to 1.78 radii. Rim lift scales from 0.055 of depth on the largest basins to 0.245 on fine fresh craters. |
| Dark smooth proximal halo | From 1.08 to 2.12 carrier radii, albedo moves toward 58% of local reflectance and smoothness rises to 0.86; the bright line field begins outside 1.24 radii. See the Tycho close-up. |

Fine crater octaves 5-9 compose after the ray cohort, so small impacts naturally cut through bright rays and clear the ray material channel on their floors/rims.

## Bounded sample budget

The hard lattice ceiling per `MoonGenerator::sample` is:

```text
10 ordinary crater octaves * at most 9 nearby cells = 90
1 sparse ray-carrier neighborhood * at most 9 cells =  9
                                                        --
maximum lattice cell visits per sample                 = 99
```

Most ordinary samples visit fewer than 9 cells because the query uses a conservative 0.56-cell reach margin. Occupancy, both position jitters, and radius share one mixed 64-bit word before the distance reject. The five coarse visible bins are immutable indexed lattice caches (2046 cells total), not a crater list/fold; sampling still visits only nearby cell identities. Octaves 5-9 remain generated statelessly on demand and represent hundreds of thousands of conceptual fine impacts without storage or population-dependent sample cost.

Maria are a separate small overlay of 41 immutable records. This is permitted because the forced redesign concerned the saturation crater fold; the mare records are spawned once by the requested clustered field.

## Performance

Same binary, pose, warm-up, and 30-frame steady measurement from `scripts/moonscape-v2-bench.play`:

| Pose | Before | Final A | Final B | Final mean | Change vs before |
|---|---:|---:|---:|---:|---:|
| Full-face orbit steady | 2.26 ms | 2.16 ms | 1.87 ms | 2.02 ms | -10.8% |
| Landed moon steady | 28.90 ms | 25.11 ms | 25.79 ms | 25.45 ms | -11.9% |

The landed `~28 ms * 1.15 = 32.2 ms` limit is cleared with 6.75 ms headroom. The independent P3 baseline script measured 26.35 ms, also above the final mean.

Cold fill did not hide a regression: full-face maximum warm-up frames average 164.4 ms final versus 155.7 ms before (+5.6%); landed maximum warm-up frames average 1843.9 ms versus 1737.1 ms (+6.2%), both within 1.15x. The cached coarse bins reduced the pre-cache landed fill from ~2.72 s to ~1.84 s without changing sample bits.

Logs:

- [Baseline fixed-pose bench](moonscape-v2/baseline/orbit-landed-bench-console.log)
- [Final bench A](moonscape-v2/final/bench-a-console.log)
- [Final bench B](moonscape-v2/final/bench-b-console.log)

## Tests and deterministic plays

`cargo test --release --lib`: **65 passed, 0 failed, 1 pre-existing ignored**.

New lunar tests cover:

- deterministic seeded lattice samples;
- bounded/unique nearby-cell visits, including cube-face ownership;
- `N proportional to D^-2` counts in a fixed probe region and >1000x small/large full-sphere count;
- mare spawn-field and record reproducibility;
- exact continuity across lattice-cell and cube-face boundaries;
- impact-center floor inheritance and dark/smooth proximal halo;
- physical albedo range and visible mare/rim range.

Final play pass, all green:

1. `physics-regressions.play`
2. `water-regressions.play`
3. `lake-regressions.play`
4. `flooded-caves.play`
5. `invariant-survey.play`
6. `auto-survey.play`
7. `camera-controls.play`
8. `solar-system.play`
9. `moon-landing.play`

`moon-landing.play` was run twice in fresh processes. Every produced PNG and JSON sidecar is byte-identical; [the SHA-256 comparison](moonscape-v2/final/moon-landing-byte-hashes.json) records all ten equal files. The suite still proves Neisor and moon gravity, landing, walking, jumping, block edits, dry lunar columns, maria material, a smooth inherited crater floor, and surviving ray material (`albedo 0.904963`).

The two intentional pinned moon-height assertions in `solar-system.play` moved from `0.134866/0.032202 km` to `0.652565/0.054191 km`; orbital assertions did not move.

## Sync diff and reels - no baseline accepted

### `sync_diff`

`sea_calib` is exactly **0.0% divergent, mean 0.0, p95 0, max 0**. The other Neisor metrics are unchanged from the standing instrument. [Full report](moonscape-v2/final/sync-diff-report.md).

### `world_reel`

Exactly two of 26 poses changed, both required moon poses:

- `moon_orbit_face`: mean delta **8.023**, changed fraction **0.2802**. Explanation: the old two blob maria / sparse face is replaced by clustered maria, basin-scale impacts, the scale-frequency field, and rare rays.
- `moon_low_flyby`: mean delta **5.167**, changed fraction **0.3285**. Explanation: crater saturation, fine overprinting, asymmetric rims, and new local height/albedo truth change the entire lunar patch.

All 24 Neisor poses produced no change finding and did not move. No `--accept` was used. [Machine report](moonscape-v2/final/world-reel-report.json).

### `sky_reel`

Only the fixed `moon_zenith` ROI changed: mean delta **0.619**, fraction over 6 **0.0231**, max delta 100. The full-frame fraction is only 0.000231 because the physical moon is tiny. The ROI detector was tightened from `1.0/5%` to `0.5/2%` so this texture-level mission change is no longer silently below threshold. Sun, sunset, dusk, and stars remain clean. No baseline was accepted.

## Evidence inventory

- [Six-panel reference contact sheet](moonscape-v2/final/contact-sheet.png)
- [Raw final teleport moon map](moonscape-v2/final/teleport-moon-map.png)
- [Teleport map before/after](moonscape-v2/final/teleport-map-before-after.png)
- [World reel lunar before/after](moonscape-v2/final/world-reel-moon-before-after.png)
- [Sky moon ROI before/after](moonscape-v2/final/sky-moon-zenith-before-after.png)
- [Deterministic feature/population probe](moonscape-v2/final/moonfeatures.log)
- [Final release-test log](moonscape-v2/final/cargo-test-release-lib.log)

The evidence play and sheet generators have no accept/bless mode.

## Code/tooling changes

- `src/moon.rs`: lattice terrain, clustered maria, asymmetric noisy crater profile, inherited floors, independent ray lines/halo, diagnostics, tests.
- `src/ui.rs`: public headless wrapper around the exact existing moon-tab synthesizer; the popup still calls the same private synthesis path.
- `examples/moonmap.rs`, `examples/moonfeatures.rs`: headless evidence and deterministic population/landmark probes.
- `scripts/moonscape-v2-bench.play`, `scripts/moonscape-v2-evidence.play`: fixed performance and acceptance poses.
- `scripts/moonscape_v2_contact_sheet.py`, `scripts/moonscape_v2_diff_sheets.py`: reproducible evidence only, no baseline mutation.
- Existing moon contact/landing/surface plays were retargeted to the new deterministic features. `solar-system.play` pins the two new intentional height samples.
- `sky_reel.py` and `sky_moon_crops.py`: texture-sensitive moon ROI and mission-correct labels.

No shader domains changed. The f64 CPU / camera-relative f32 GPU boundary remains intact. `voxel.rs` was not forked or changed; it continues to sample the exact same `MoonGenerator` as mesh terrain.

## Input/worktree notes

The mission named `viewer/interchange/codex/SOLAR-P2-FINDINGS.md` and `SOLAR-P3-FINDINGS.md`, but neither file exists in this checkout or Git history. Their operative contracts were recovered from commits `afa6755` and `a868f29`, landed source, scripts, and commit messages. The pre-existing untracked root `run.err` and `run.log` were left untouched.

No commit was made.
