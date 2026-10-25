# Biome blending round 3 findings

Branch: `sol/blend-r3`  
Base/HEAD: `93be45c6528eeeac1679015dfb6ec95b7c667bfe`  
World: `output/seed42_r8`  
Commit made: **no**

## Verdict

Andrew's two annotated failures are addressed by two deliberately separate
range treatments:

1. The biome-domain displacement is now five octaves of seamless 3-D value
   noise, from continental/intermediate scale down through sub-kilometre
   detail. The 189.542 km snow/grass silhouette is no longer a sequence of
   similarly warped raster cells: its lobes, notches, holes, and wisps vary at
   several scales and do not expose a repeatable baked-cell outline.
2. Mesh terrain carries a range-only, area-preserving boundary appearance.
   Its cross-material zone is up to 8 km wide and uses 56 km + 14 km structure
   ahead of the already-approved 7 km-to-metre comparator. At the 15.734 km
   pose, the former one-pixel razor is a deep, irregular patch field rather
   than a line.

The public/local path is intentionally separate. The approved 300 m,
four-scale local category treatment remains the result of the public
`climate_surface` API. Mesh fragments hand off continuously from local to
range appearance over 4-12 km camera-relative distance. A unit test compares
the public local result with the local half of the dual result bit-for-bit.

## Review frames

The reproducible script is
[`viewer/scripts/biome-blend-round3.play`](viewer/scripts/biome-blend-round3.play).
Weather is off and the sun is pinned at the values from Andrew's sidecars.

Final frames:

- [`r3_warped_tiling.png`](viewer/interchange/runs/biome-blend-round3/r3_warped_tiling.png)
- [`r3_razor_boundary.png`](viewer/interchange/runs/biome-blend-round3/r3_razor_boundary.png)
- [`r3_approved_orbit.png`](viewer/interchange/runs/biome-blend-round3/r3_approved_orbit.png)
- [`r3_approved_19km.png`](viewer/interchange/runs/biome-blend-round3/r3_approved_19km.png)
- [`r3_approved_1km.png`](viewer/interchange/runs/biome-blend-round3/r3_approved_1km.png)

Pre-change captures from the same script are in
`viewer/interchange/runs/biome-blend-round3-baseline/`.

Visual review of the three previously approved poses:

- 30.568, 27.529 orbital: the three-island continental breakup and coastline
  remain recognizable; only their land-material margins gain the intended
  multiscale breakup.
- 31.152, 28.039 at 19 km: the broad desert interface remains coherent, but
  its edge is now a hierarchy of patches rather than a thin raster trace.
- 31.153, 28.010 at 1.2 km: the approved local speckle and silhouette remain
  on the original 300 m/four-octave path. The packed-range optimization did
  not alter this PNG at all.

Two complete final renders were byte-identical. SHA-256:

| frame | SHA-256 |
|---|---|
| `r3_warped_tiling.png` | `BFFA75A8C0C414017E99EB84DAFE4B477CF5C75F5E11640E4C9CFC1D87EA8574` |
| `r3_razor_boundary.png` | `377D7CA97C8D2B0F1E7E1777AEE51C95F6092EEBCA8161CBE3325632A30378AC` |
| `r3_approved_orbit.png` | `F38C6CCD5C7F5C079BBAD5B7A7F880833B518866C6328DEB63D5D8D62C659759` |
| `r3_approved_19km.png` | `AA216A6BCD4A7E8B0ADF2AEAAD282FCD408A43E1154CA7C1D7A51BB62308E459` |
| `r3_approved_1km.png` | `551DBD90B3FC1FE8FDBC8983AA824C0A185083EE325B869B11318F4B5BCDB8A2` |

## Andrew's knobs

All biome-domain and boundary-field knobs are together near the top of
[`viewer/src/planet.rs`](viewer/src/planet.rs). Range shading/storage knobs are
in [`viewer/src/terrain.rs`](viewer/src/terrain.rs), and the distance handoff is
in [`viewer/src/shader.wgsl`](viewer/src/shader.wgsl).

| knob | value | effect |
|---|---:|---|
| `CROSS_BLOCK_ECOTONE_KM` | `0.300` km | Unchanged local full-width category band. |
| `BIOME_WARP_OCTAVES` | `5` | Number of seamless displacement scales. |
| `BIOME_WARP_BASE_AMPLITUDE_TEXELS` | `0.65` | Broadest octave displacement strength. |
| `BIOME_WARP_BASE_WAVELENGTH_TEXELS` | `5.0` | Broadest wavelength; about 85 km at a Neisor face center. |
| `BIOME_WARP_LACUNARITY` | `3.6` | Divides wavelength per octave; higher packs in finer detail faster. |
| `BIOME_WARP_PERSISTENCE` | `0.36` | Multiplies finer-octave amplitude; higher makes fine crinkle stronger. |
| `BIOME_WARP_MAX_DISPLACEMENT_TEXELS` | `1.20` | Caps total displacement while allowing cell topology to break. |
| `BIOME_BOUNDARY_ZONE_KM` | `8.0` km | Maximum range-visible cross-material zone width. |
| `BIOME_BOUNDARY_ZONE_MAX_TEXEL_FRACTION` | `0.90` | Metric-safe per-axis cap near face edges/corners. |
| `BIOME_BOUNDARY_PREFILTER_SIDE_WEIGHT` | `0.15` | Symmetric `[0.15, 0.70, 0.15]` range-only categorical prefilter. |
| `BIOME_BOUNDARY_FIELD_COARSE_OCTAVES` | `2` | Broad layers added only to the range comparator. |
| `BIOME_BOUNDARY_FIELD_BASE_PATCH_COLUMNS` | `32768` | About 56 km broad range patches. |
| `BIOME_BOUNDARY_FIELD_LACUNARITY` | `4.0` | Produces the second range layer at about 14 km. |
| `BIOME_BOUNDARY_FIELD_PERSISTENCE` | `0.55` | Relative strength of that 14 km layer. |
| `ECOTONE_FIELD_OCTAVES` | `4` | Approved local comparator; unchanged. |
| `ECOTONE_BASE_PATCH_COLUMNS` | `4096` | Approved broad local layer, about 7 km; unchanged. |
| `ECOTONE_LACUNARITY` | `16.0` | Approved 7 km/440 m/27 m/1.7 m spacing; unchanged. |
| `ECOTONE_PERSISTENCE` | `0.5` | Approved local fine-layer strength; unchanged. |
| `BIOME_RANGE_CATEGORICAL_CONTRAST` | `0.45` | `1` is fully categorical; `0` is fully continuous expected coverage. |
| `BIOME_RANGE_BLEND_START_KM` | `4.0` km | Distance where local-to-range handoff begins. |
| `BIOME_RANGE_BLEND_END_KM` | `12.0` km | Distance where the range treatment is complete. |
| `BIOME_RANGE_COLOR_DELTA_SCALE` | `0.5` | Storage scale, not an art-width knob; gives <0.0021 reconstruction error. |

Approximate warp wavelength/amplitude pairs on Neisor are
85/11, 24/4.0, 6.5/1.4, 1.8/0.51, and 0.50/0.18 km before the aggregate
1.20-texel cap.

## Invariants and implementation notes

- Sea ownership is resolved at the unwarped world position before any biome
  displacement. True sea returns immediately. A dry `255` climate sentinel
  can participate in land appearance, but a warp whose target is physical sea
  is rejected. `biome_warp_cannot_cross_true_sea` and
  `dry_ocean_sentinel_participates_in_biome_warp` both pass.
- `climate_surface` and `koppen` public signatures are unchanged. No public
  climate-surface/Koppen API was added or removed. The range pair is
  `pub(crate)`.
- Symmetric ramps and the symmetric prefilter preserve expected category area.
  Both local and range comparators use a normal-CDF probability transform.
- Both comparator fields are one value per canonical column, deterministic on
  repeat, and bit-exact across cube-face edges/corners.
- The range color is packed as a half-scale `SNORM8x4` delta. Zero is exact for
  water, voxels, and impostors. This restored `Vertex` to its original 72-byte
  footprint after an initial float-RGB version exhausted GPU memory late in
  the 24-pose reel. The final unmodified reel completes.

Area-fraction samples (target 0.25/0.50/0.75, tolerance 0.008):

| field/seed | measured occupancy |
|---|---|
| local / 247492824 | 0.24995 / 0.49870 / 0.75020 |
| local / 23096 | 0.25127 / 0.50140 / 0.75171 |
| range / 247492824 | 0.24814 / 0.49677 / 0.75031 |
| range / 23096 | 0.25261 / 0.50427 / 0.75188 |

The synthetic range-zone continuity test also verifies pure ownership outside
the support, exactly 0.5 at the old boundary, and antisymmetry to `1e-12`.

## Gate results

| gate | result |
|---|---|
| `cargo build --release --example play --example tile_cost` | PASS |
| `cargo test --release --lib` | PASS, 29/29 |
| `cargo test --release` | PASS, 29 unit + 1 golden parity test; doc tests pass |
| `cargo test --release --lib categorical_comparators_preserve_area_fractions -- --nocapture` | PASS; measurements above |
| Seven play suites | PASS: physics, water, lake, flooded caves, invariant survey, auto survey, camera controls |
| Five-pose render, repeated | PASS; all hashes identical |
| `python scripts/sync_diff.py --label blend-r3-final` | PASS; `sea_calib` 0.0%, mean/p95/max 0 |
| Tile cost | PASS; final medians 0.257/0.259/0.253 s, median-of-medians 0.257 s |
| `python scripts/world_reel.py` | Completed; four expected change flags explained below; no VOID finding |
| Difficulty Lake colcensus | `LIP 0`, `EDGE 0` |
| Pond ledger colcensus | Inherited P-1 debt: `LIP 336`, `EDGE 219`; explained below |
| `census --lips` | Completed: `WALL 0`, `JUMP 0`, `SEAJUMP 0`; inherited LIP cohorts below |
| `git diff --check` | PASS (Git only prints configured LF-to-CRLF notices) |

### Tile-cost delta

Recorded warmed baseline medians were 0.257 and 0.256 s. The final
median-of-medians is 0.257 s, approximately +0.2% against their midpoint and
well below the +8% ceiling. Fixed-set checksums:

| checksum | baseline | final | meaning |
|---|---:|---:|---|
| vertices | `746031` | `746031` | exact |
| geometry | `9233981608705220` | `9233981608705220` | exact |
| shore | `1514547866741203` | `1514547866741203` | exact |
| other | `5073609388707703` | `5130752995887994` | expected color/range payload change |

## World reel: every emitted flag

Invocation was exactly `python scripts/world_reel.py` from `viewer/`, with no
flags. It rendered, linted, and compared all 24 poses against
`viewer/interchange/reel/accepted`. The current metrics are in
[`viewer/interchange/reel/report.json`](viewer/interchange/reel/report.json).
The accepted baseline was **not** changed.

| emitted finding | metrics | explanation |
|---|---|---|
| `coast_beach` | mean 1.785, changed fraction 0.0001 | Low-altitude local blocks/terrain geometry are visually unchanged. The new warped climate lookup produces small sub-threshold tint differences over the frame; only 0.01% of pixels exceed delta 6. No water or coastline ownership moved. |
| `jungle` | mean 1.360, changed fraction 0.0202 | The near voxel forest, lake, and shoreline remain intact. The changed pixels are the distant horizon's sand/forest interface entering the 4-12 km range handoff, which is intended. |
| `orbital_coast` | mean 1.789, changed fraction 0.0537 | Desert/grass patches on land are less cell-like and their margins are softer. Ocean pixels and the physical coastline remain fixed by pre-warp sea gating. |
| `orbital_biomes` | mean 4.517, changed fraction 0.1272 | This is the intended high-range change: snow/grass and grass/sand edges acquire broad mixing plus fractal detail instead of baked-cell stair steps. |

There were no VOID lint findings. GRID scores are informational in this reel
version and were not emitted as findings. All four emitted lines above are
accounted for.

For completeness, the script's optional command-line flags mean:

- `--skip-render`: lint/diff already-existing current frames. Not used, because
  it would not exercise this build's renderer.
- `--accept`: copy current frames over the accepted baseline. Not used; the
  contract reserves acceptance for Andrew/human sign-off.

## Water-contract inherited findings

The pond ledger command at 12.199, -44.827 exits 2 with 336 LIP and 219 EDGE
columns. `WATER-CONTRACT.md` already lists P-1 residual pond walls in this same
pond country. The exhaustive census likewise reports 61,360 LIP findings,
including 740 liquid-lake and 576 one-block liquid-lake findings, while WALL,
JUMP, and SEAJUMP are all zero. The contract separately lists the inherited
W-7/W-5b mega-lake families.

This mission does not change `terrain::sample`, height generation, shoreline
fields, lake/river decisions, or voxel water construction. The exact geometry
and shore checksums above, `sea_calib` 0.0%, and the reel's unflagged
`dossier_ponds` pose corroborate that these cohorts did not rise because of
biome blending. They are reported here rather than silently treated as green,
and were not expanded into an out-of-scope water fix.
