# W4 structural seasonality findings

Date: 2026-07-12  
Branch/worktree: `sol/w4-seasons` at `d90835d`  
World: `output/seed42_r8`  
Commit: none made

## Outcome

W4 is implemented as one immutable structural-season input shared by terrain,
mesh tiles, voxel chunks, physics, edits, the map, and test instruments.
`season_frac` is Neisor's orbital mean anomaly from `orbits.rs`; W4 creates no
second clock and does not read synoptic weather.

- `WeatherTuning`: `freeze_c=-5`, `thaw_c=-2`, `season_buckets=24`, and
  `deciduous_tint_strength=0.22` are validated knobs.
- `StructuralSeason` quantizes the orbital phase once. The canonical bucket
  containing `0.45` uses the legacy annual raster and `temp < -4` class
  exactly. Weather-off selects the same annual law. Every other bucket reads
  the baked Fourier temperature. Phase sine/cosine is precomputed once per
  bucket; the sample hot path is an interleaved bilinear coefficient read plus
  fmadds.
- `weather.bin` is WEA3: the WEA2 layers plus twelve monthly sea-ice rasters.
  Inland water follows Fourier temperature hysteresis; the sea follows cyclic
  interpolation of `seaice_monthly`, dithered by the stable ecotone comparator.
- Cooling uses the freeze edge and warming uses the thaw edge. Each contour is
  dithered over 1 C by the existing world-anchored ecotone comparator.
- `Sample.frozen` is the single class bit. Lakes, rivers, ponds, sea ice,
  flooded-karst water, frost/ice material, shoreline paint, collision support,
  ceilings, and immersion consume it.
- `GpuTile` carries its build bucket. On a bucket change, old geometry remains
  visible while selected stale work is replaced (at most 8 tiles and 16 chunks
  queued per normal frame). Captures deliberately settle all staged work.
- Player physics receives an immutable `SeasonalPlanet` with the same bucket
  as chunk workers. This is the proof path behind the winter-walk/summer-swim
  assertions below.
- Positive column edits anchor above the visible water/ice top. The edit stays
  dry through later thaw/freeze changes. Per-body edit separation is unchanged.
- Snow override and grass/surface tints use seasonal temperature. Broadleaf
  tint uses local temperature plus its derivative (cooling autumn versus
  warming spring). Annual vegetation ownership, lottery, species, treeline,
  and positions remain unchanged; the unit gate compares winter and summer
  tree positions directly.
- The planet boundary-zone/range machinery and weather presentation layer were
  not changed.

## Automated gates

| Gate | Result |
|---|---|
| `cargo test --release --lib` | PASS: 71 passed, 1 ignored evidence probe |
| existing asserting play suites | PASS: 8/8 (`physics`, `water`, `lake`, `flooded-caves`, `invariant`, 69-probe `auto-survey`, `camera-controls`, `solar-system`) |
| `seasonal-cycle.play` | PASS twice; all four PNG hashes byte-identical |
| world reel | PASS: all 26 prior poses unchanged/clean plus 2 new summer poses clean (28 total), no `--accept` |
| `git diff --check` | PASS |

New release unit gates cover hysteresis monotonicity, bucket determinism,
WEA3 monthly sea ice, weather-off and canonical `sample()` bit equivalence,
placed-edit precedence, and season-independent tree positions.

`seasonal-cycle.play` proves, at the same `58.79455, 87.221664` column:

- winter bucket 22 (`0.9375`): `-5.574295 C`, frozen, grounded on ice,
  not underwater, and no liquid water query;
- summer bucket 12 (`0.520833`): `-0.922170 C`, thawed, `has_water=true`,
  and underwater;
- autumn broadleaf probe: RGB changes from base `(0.065, 0.200, 0.035)`
  to `(0.113372, 0.192964, ...)` without moving trees;
- after one positive edit, `edited_water_count=0` in both winter and summer.

Double-run SHA-256 values:

| Frame | SHA-256 |
|---|---|
| `winter_walk` | `B193035EACE0E9597D595DF86BB987B17CF83028364B58F3B97EB973EE4F4514` |
| `summer_swim` | `964A49738DA2FEB34A35D7CCC2CAAE690EC21CCA8E448A4A58E4170EF517893B` |
| `deciduous_tint` | `BCBA52DCE7936BE1A9E280193707E476B6B90441A6729B9B4645AAD3BCF72ACC` |
| `edit_override_summer` | `1365110BAB4C88590CE5A16E0ADDF7073AEF16B1838644DF6B87284DBB836D3F` |

## Mesh/voxel sync-diff

Canonical `sea_calib` remains exactly `0.0%`. Deep winter also remains exactly
`0.0%`. The requested frozen-lake pose agrees more closely in winter, while
the established non-seasonal pose deltas remain in their prior ranges.

| Pose | Canonical divergent / mean d | Winter divergent / mean d |
|---|---:|---:|
| river_low | 34.4% / 12.35 | 34.7% / 12.18 |
| river_mid | 33.1% / 11.38 | 33.4% / 11.23 |
| lake_shore | 26.8% / 69.24 | 26.8% / 69.20 |
| savanna | 41.3% / 7.55 | 41.2% / 7.55 |
| beach | 58.4% / 7.85 | 59.4% / 7.90 |
| jungle | 46.7% / 19.91 | 46.7% / 19.91 |
| taiga | 12.0% / 13.49 | 10.9% / 13.03 |
| desert | 46.2% / 6.74 | 46.3% / 6.73 |
| peak | 27.9% / 12.12 | 27.9% / 12.12 |
| valley | 38.3% / 12.53 | 38.1% / 12.48 |
| groves | 50.6% / 12.94 | 50.6% / 12.86 |
| ice_top | 55.8% / 8.76 | 52.1% / 7.21 |
| frozen_lake | 59.2% / 19.54 | 40.3% / 7.98 |
| karst | 42.6% / 63.84 | 42.6% / 63.81 |
| **sea_calib** | **0.0% / 0.0** | **0.0% / 0.0** |

Reports: `viewer/interchange/runs/sync-diff/report.md` and
`viewer/interchange/runs/sync-diff-season-0.95/report.md`.

## Water-contract census

The canonical cut is the legacy sample law, so its current-baseline cohorts
are unchanged. Winter changes totals slightly as expected, but W1/W2 remain
zero-regression and the final reel supplies W5's no-void evidence.

| Season | WALL | JUMP | SEAJUMP | LIP | liquid-lake | 1-block liquid-lake |
|---|---:|---:|---:|---:|---:|---:|
| canonical `0.45` (bucket 10) | 0 | 0 | 0 | 61,344 | 740 | 576 |
| requested `0.95`, sampled `0.9375` (bucket 22) | 0 | 0 | 0 | 61,462 | 742 | 578 |

Both full runs sampled 221,841,122 points. Reports:
`viewer/interchange/census-w4-canonical.md` and
`viewer/interchange/census-w4-winter.md`.

Ledger-site block truth was stable at both seasons:

| Site | Season | LIP | EDGE | SHOAL | CAVEP |
|---|---|---:|---:|---:|---:|
| Difficulty Lake | canonical | 0 | 0 | 30,069 | 462 |
| Difficulty Lake | winter | 0 | 0 | 30,069 | 462 |
| pond site | canonical | 88 | 0 | 0 | 0 |
| pond site | winter | 88 | 0 | 0 | 0 |

The pond's 88 one-block LIPs are inherited ledger debt; W4 did not increase
them. W3 structure ownership follows the same class: liquid-only apron/shore
material disappears with frozen water and returns on thaw. W4 did not create
dry residual structures.

## Visual evidence and reel notes

- Four-season fixed-lake sheet:
  `viewer/interchange/w4-evidence/frozen_lake_four_seasons.png`.
  Seasons 0.00/0.25 are a continuous ice sheet; 0.50/0.75 expose liquid at
  the same shoreline.
- Monthly sea-ice edge sweep:
  `viewer/interchange/w4-evidence/sea_ice_edge_sweep.png`.
  The 0.50/0.5417 panels show the baked raster retreat; 0.00/0.25/0.75 show
  the re-advanced sheet.
- The new `sea_ice_summer` reel frame visibly contains broken liquid leads at
  the same pose as `sea_ice`.
- The inherited `frozen_lake` reel pose at `51.180, 85.830` does not actually
  frame visible lake water in its accepted canonical image. Its required
  same-pose summer twin is consequently clean but visually unchanged. This is
  a pre-existing pose-label/aim defect, not a seasonal class failure; the
  working fixed-lake evidence and physics suite use `58.79455, 87.221664`.

No baseline was accepted or modified.

## Performance

Original pre-W4 `perfbench.play` versus final W4, same machine/process shape:

| Pose | Before avg / p95 | After avg / p95 | Avg ratio | p95 ratio |
|---|---:|---:|---:|---:|
| polar ground/ice | 19.20 / 26.65 ms | 17.87 / 24.82 ms | 0.931x | 0.931x |
| forest ground | 36.48 / 57.89 ms | 24.64 / 46.25 ms | 0.675x | 0.799x |

The paired 611-tile build probe (746,031 vertices, same geometry checksum)
measured `0.306 s` annual versus `0.309 s` deep winter, `1.010x`. The
interleaved temperature coefficients and precomputed bucket phase keep the
Fourier hot path inside the 1.05x budget.

## Expected seam

The sea-ice monthly raster is authoritative for sea and Fourier temperature is
authoritative for lakes/rivers/ponds. At a coast where those baked truths
disagree, a frozen/liquid class seam can be visible across the ownership line.
The stable ecotone dither breaks up the contour, but intentionally does not
override the sea raster. The evidence sweep shows this as patchy edge retreat;
it is the specified truth boundary, not mesh/voxel disagreement.
