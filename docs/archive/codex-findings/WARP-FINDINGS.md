# Continental biome domain warp — findings

- Date: 2026-07-11
- Branch/worktree: `sol/biome-warp` at starting point `2b2d921`
- World: `output/seed42_r8` / `viewer/assets`

## Outcome

The baked Köppen lookup now uses one deterministic, face-seamless domain-warped climate position. At the reported 288 km pose, the old 40 km rectangular staircases break into nested coastline-like lobes and teeth. The same transform owns mesh shading, voxel biome/material inputs, tree class reads, the photo map's biome layer, probes, and every other public Köppen consumer.

The existing 300 m categorical ecotone and four-scale local comparator were not retuned. Their constants, world-column phase, area fractions, and continuity behavior are unchanged; only the large-scale boundary position feeding them moves.

## Exact cut: what warps

`Planet::climate_position` in `viewer/src/planet.rs` is the single position transform.

- `Planet::koppen` returns the class at that position. All direct consumers therefore agree: voxel `ColCtx`, voxel/tree species, mesh tree impostors, play/probe output, examples, and the photo map.
- `climate_surface` uses the same position for nearest class, the four-texel material weights, the smoothed Köppen hue/forest signature, annual temperature, and annual precipitation. Mesh and voxel ground materials/tints therefore remain coherent.
- The visual snowline inside `climate_surface` uses the warped annual temperature. Its approved metre-scale comparator remains anchored to the original world column.
- The photo map's biome layer calls `biome_climate`, so its class and temperature-driven relief use one warped position. Its elevation relief remains unwarped.
- Broad same-class interiors return the identity position. A precomputed 8-neighbor edge bit proves that a displacement capped below one local raster texel cannot change class there; class and tint both use that same identity position. This avoids procedural work with no categorical effect.

## Exact cut: what does not warp

- elevation, roughness, terrain detail, erosion/carving, and geomorph geometry;
- the raw and blurred ocean masks, `water_frac`, and the `terrain::sample` sea decision;
- rivers, lakes, ponds, cave water, beaches, shores, frozen/liquid water rules, and every other water rule;
- the unwarped `Planet::temp`, `Planet::precip`, and combined `temp_precip` values used by terrain, hydrology/freeze decisions, ecology/treelines, and weather;
- temperature and precipitation photo-map base layers, including live/seasonal weather sampling;
- the approved local comparator's four scales, seed streams, canonical column snapping, and 300 m band.

The ocean gate runs before noise. A source ocean texel returns class 255 without evaluating the warp. If a land point's candidate warped position lands on an ocean texel, the entire climate lookup falls back to the original land position. Thus the warp cannot move land classes into open sea or move the raw sea boundary inland.

## Andrew tuning knobs

All large-scale knobs are grouped at the top of `viewer/src/planet.rs` beside the existing local knobs.

| knob | final value | meaning |
|---|---:|---|
| `BIOME_WARP_OCTAVES` | 2 | direction-space vector-noise layers |
| `BIOME_WARP_BASE_AMPLITUDE_TEXELS` | 0.40 | broad displacement amplitude |
| `BIOME_WARP_PERSISTENCE` | 0.45 | finer amplitude multiplier; amplitudes are 0.40, 0.18 texel |
| `BIOME_WARP_BASE_WAVELENGTH_TEXELS` | 4.0 | broad wavelength |
| `BIOME_WARP_LACUNARITY` | 8.0 | finer frequency multiplier; wavelengths are 4.0, 0.5 texel |
| `BIOME_WARP_MAX_DISPLACEMENT_TEXELS` | 0.70 | strict combined displacement cap in local raster texels |
| `BIOME_WARP_SEED_OFFSET` | `0x0B10_0A2E` | independent deterministic stream |

At a Neisor face center (`radius_km=8660.254`, `res=1024`), the wavelengths are approximately 68 km and 8.5 km; nominal octave amplitudes are approximately 6.8 km and 3.0 km. A symmetric gnomonic metric factor keeps the 0.70 cap valid at face edges/corners without introducing face axes or seams.

The approved local targets remain:

| knob | unchanged value |
|---|---:|
| `CROSS_BLOCK_ECOTONE_KM` | 0.300 km |
| `ECOTONE_FIELD_OCTAVES` | 4 |
| `ECOTONE_BASE_PATCH_COLUMNS` | 4096 |
| `ECOTONE_LACUNARITY` | 16 |
| `ECOTONE_PERSISTENCE` | 0.5 |
| `KOPPEN_HUE_NUDGE` | 0.14 |

## Implementation/performance notes

- Each octave is a C2-continuous quadrature vector field evaluated from unit world direction and seed. One phase/floor supplies both vector channels; there are no per-vertex lattice-corner hashes.
- The vector is projected onto the sphere tangent. Exact cube-edge ownership is canonicalized before evaluation.
- Climate edge flags are packed one bit per raster texel. Same-class interiors skip all warp evaluations.
- Temperature and precipitation retain their original values and coordinates but are stored interleaved and bilinearly sampled together. Terrain reads them in one pass, and warped tint reads need one cache stream rather than two.
- Raster coordinates are computed once and reused by the class, signature, local weights, and tint reads.

## Performance gate

`tile_cost` uses the fixed warmed 611-tile lake-shore set (746,031 vertices), five timed runs, and a median.

| build | five runs (s) | median | delta |
|---|---|---:|---:|
| untouched `2b2d921` | 0.257, 0.239, 0.241, 0.261, 0.243 | 0.243 s | — |
| final run A | 0.272, 0.265, 0.266, 0.263, 0.272 | 0.266 s | +9.5% |
| final run B | 0.258, 0.271, 0.261, 0.261, 0.261 | 0.261 s | +7.4% |
| final run C | 0.262, 0.269, 0.265, 0.269, 0.256 | 0.265 s | +9.1% |

All three final medians are below the +10% gate. The host showed scheduler variance during development, so consecutive final measurements are reported rather than selecting one.

Checksums prove the cut:

| channel | untouched | final | result |
|---|---:|---:|---|
| vertices | 746031 | 746031 | identical |
| geometry | 9233981608705220 | 9233981608705220 | identical |
| shore | 1514547866741203 | 1514547866741203 | identical |
| appearance/other | 5073611354040850 | 5073612703153648 | changed as intended |

Required release build command passed:

```text
cargo build --release --example play --example tile_cost
```

## Visual evidence

The repeatable script is `viewer/scripts/biome-warp-evidence.play`; outputs are under `viewer/interchange/runs/biome-warp-evidence/`.

| capture | pose | observation |
|---|---|---|
| `reported_orbit_288km.png` | 42.765, 38.439, 288.186 km, yaw -64, pitch -85 | reported raster rectangles become nested irregular lobes/teeth |
| `savanna_orbit_288km.png` | 5.560, 31.741, 288 km | independent warm-continent orbital check |
| `jungle_orbit_288km.png` | -0.906, -67.804, 288 km | independent equatorial continent plus unwarped coast check |
| `reported_corner_10km.png` | 16.684, -23.367, 10 km | large-scale curve/island structure overlaps the local scale |

`viewer/scripts/fractal-dither-evidence.play` was rerun in full: all nine reported-corner, desert/grass, and snowline captures completed at walk, 400 m, and 3 km. Boundary positions moved, as intended. The local patch field and snowline retain their approved nested/speckled character.

Two consecutive final executions of the orbital evidence script produced byte-identical PNGs:

```text
010891cdaf03482980b94d8ea8fd2b93c97084402beda17fa2ac3be41228b28a  jungle_orbit_288km.png
ebc911a837e8087588ea2ea389523750a821039deb4140d68206890d3877a965  reported_corner_10km.png
d98dc035d61a4a3d9a8d440fe4e7cc6e1caed1480f0cbd7fabce6106e1c13d60  reported_orbit_288km.png
bd0589024ecdBfeaa67c7019cfd0741035f05df1ed41cb1d1889839fd7d54b56  savanna_orbit_288km.png
```

## Automated gates

- `cargo test`: 22 unit tests passed, plus the Python/Rust noise golden; zero failures.
- New `biome_warp_is_deterministic_and_exact_across_cube_faces`: passed bit-exact repeat and shared edge/corner checks.
- New `biome_warp_cannot_cross_the_raw_ocean_mask`: passed 98,304 sampled synthetic positions.
- Existing local area-fraction, canonical-column, cube-seam, texel-corner continuity, ocean-seam, lake/shore, and morph gates: passed unchanged.
- Seven release play suites: `physics-regressions`, `water-regressions`, `lake-regressions`, `flooded-caves`, `invariant-survey`, generated `auto-survey` (69 probes / 7 classes), and `camera-controls`: all passed.
- `git diff --check`: passed.

Final mesh/voxel sync-diff standing table:

| pose | divergent | mean d | p95 d | lum bias |
|---|---:|---:|---:|---:|
| river_low | 33.6% | 12.5 | 30 | -7.9 |
| river_mid | 31.9% | 11.6 | 29 | -7.1 |
| lake_shore | 22.0% | 11.8 | 55 | -0.3 |
| savanna | 40.9% | 7.35 | 22 | +0.2 |
| beach | 59.1% | 7.86 | 22 | -3.2 |
| jungle | 43.5% | 19.2 | 61 | +1.9 |
| taiga | 11.5% | 7.78 | 29 | -0.9 |
| desert | 45.3% | 6.77 | 15 | -0.5 |
| peak | 27.1% | 12.58 | 38 | -6.7 |
| valley | 35.9% | 12.05 | 34 | +4.2 |
| groves | 46.0% | 11.54 | 30 | +2.9 |
| ice_top | 55.8% | 8.76 | 19 | +3.4 |
| karst | 36.0% | 11.84 | 51 | -0.3 |
| **sea_calib** | **0.0%** | **0.0** | **0** | **+0.0** |

No commit was attempted. Pre-existing untracked `run.err` and `run.log` were left untouched.
