# Blend round 4B findings

## Verdict

The round-4 verdict was wrong about the primary 9 km specimen. The climate
reconstructor was running, but it did not own the visible tan wash. A separate
generic-beach coverage ramp was mixed into every categorical endpoint after
reconstruction, so it restored a smooth sand-to-grass gradient over the same
multi-kilometre region.

That material owner is now categorical at range. `blend_9km` changes from the
untouched baseline by mean 5.870, 21.854% of pixels over 6, and max 96. The
reported wash is a discrete green/sand ownership field rather than the former
gentle tint. The round-4 95 km frame remains byte-identical, and the reported
288 km control moves only 0.0058 mean from round 4 while retaining its 0.45
orbital contrast. Water and landscape truth remain exact baseline matches.

## Exact 9 km transect probe

The ignored evidence test `reported_9km_range_transect` ray-casts screen pixels
from `(650,120)` (tan) to `(950,500)` (green) through the photographed camera,
iterates the terrain intersection, and calls the same climate-position and
8 km coverage functions used by mesh vertices. Channels are the fixed semantic
order: six grass-greenness families, sand, snow.

| # | Pixel | Lat / lon | Koppen | Coverages c0..c7 | Temp C | Precip mm | Vertex path | Beach coverage / fine gain |
|---:|---:|---|---:|---|---:|---:|---|---|
| 1 | 650,120 | 52.995778 / 99.959072 | 25 | 0 / 0 / .0045 / 0 / .9955 / 0 / 0 / 0 | 7.264 | 1743.6 | `range_edge` complete | .5059 / .0784 |
| 2 | 683,162 | 52.998949 / 99.968005 | 25 | 0 / 0 / .0057 / 0 / .9943 / 0 / 0 / 0 | 7.243 | 1714.9 | `range_edge` complete | .2549 / .0784 |
| 3 | 717,204 | 53.002070 / 99.976719 | 25 | 0 / 0 / .0068 / 0 / .9932 / 0 / 0 / 0 | 7.225 | 1689.2 | `range_edge` complete | .3020 / .0784 |
| 4 | 750,247 | 53.005141 / 99.985280 | 25 | 0 / 0 / .0101 / 0 / .9899 / 0 / 0 / 0 | 7.196 | 1637.2 | `range_edge` complete | .1922 / .0784 |
| 5 | 783,289 | 53.008162 / 99.993705 | 25 | 0 / 0 / .0116 / 0 / .9884 / 0 / 0 / 0 | 7.182 | 1615.6 | `range_edge` complete | .0980 / .0784 |
| 6 | 817,331 | 53.011108 / 100.002011 | 25 | 0 / 0 / .0141 / 0 / .9859 / 0 / 0 / 0 | 7.164 | 1583.9 | `range_edge` complete | .0196 / .0784 |
| 7 | 850,373 | 53.014005 / 100.010164 | 25 | 0 / 0 / .0156 / 0 / .9844 / 0 / 0 / 0 | 7.157 | 1567.7 | `range_edge` complete | 0 / .0784 |
| 8 | 883,416 | 53.016826 / 100.018171 | 25 | 0 / 0 / .0169 / 0 / .9831 / 0 / 0 / 0 | 7.143 | 1547.3 | `range_edge` complete | 0 / .0784 |
| 9 | 917,458 | 53.019605 / 100.026027 | 25 | 0 / .0004 / .0231 / 0 / .9764 / 0 / 0 / 0 | 7.107 | 1509.2 | `range_edge` complete | 0 / .0784 |
| 10 | 950,500 | 53.022301 / 100.033723 | 25 | 0 / .0009 / .0290 / 0 / .9701 / 0 / 0 / 0 | 7.094 | 1489.4 | `range_edge` complete | 0 / .0784 |

### Diagnosis of the three candidates

- The grass buckets are coarse here, but the fast-path hypothesis is false:
  every point is marked `range_edge` and runs the complete 4x4 coverage path.
  The transect is 97.0--99.6% family 4 with a small family-2 contribution.
  Koppen stays 25 throughout. Precipitation is already above the grass tint's
  wet saturation limit, so the 255 mm precipitation change is not the tan
  ramp's color owner.
- Climate endpoint amplitude was modest (pre-fix active spans were 13--43
  bytes), but increasing it would have attacked the wrong signal. The unlit
  vertex color still moved from `[80,85,42]` to `[18,48,13]` because
  `beach_frac` fell from 0.506 to zero and was applied identically to all
  endpoints. The missing categorical sand/grass span is 128--129 bytes.
- The unresolved climate tail is not responsible. It remains 0.1264 of the
  climate variance. At this calm site the beach fine gain is 0.0784; the
  range prefix agrees with the full beach field's half-space 99.387% of the
  time (92.569% at the rugged endpoint). There is no five-octave smooth tail
  large enough to explain the photographed ramp.

This is therefore not a homogeneous in-biome art-direction question. It is a
mixed grass/beach material interface that bypassed the range category path.

## Implementation

- Generic beach has a four-byte terrain payload: coverage, adaptive fine gain,
  validity, and one reserved byte. Climate thresholds remain unchanged.
- The exact block beach comparator and its WGSL twin now use one calibrated
  gradient lattice. Blocks resolve all four established carriers; fragments
  resolve the 7 km and ~440 m prefix. The ~440 m carrier fades from 120--240 km
  before orbit can undersample it; the broad 7 km owner remains.
- Climate deviations and the independent beach deviation are centered jointly.
  Their expected RGB is exactly the former continuous mean, so categorical
  restructuring does not move broad color or the orbital 0.45 target.
- `Vertex` grows from 100 to 104 bytes. The four adjacent scalar attributes are
  exposed as one `Float32x4`, staying within the GPU attribute limit. The
  teleport-batch LRU remains unchanged.
- Ownership rules and knobs are recorded in `TRANSITIONS.md` under "Round 4B."

## Before/after contact sheet

The baseline remains the untouched `c334289` capture. Click a frame for its
original 1280x720 PNG; each table cell carries its full SHA-256 stamp.

| Pose | Before (`c334289`) | After round 4B |
|---|---|---|
| 9.07 km | [![9 km before](viewer/interchange/runs/blend-r4/before/blend_9km.png)](viewer/interchange/runs/blend-r4/before/blend_9km.png)<br>`D9BCAE29D2214CABD261ADCDADB9293AB3D8FD9BD5CBB6957328520AFD758DF2` | [![9 km after](viewer/interchange/runs/blend-r4/after/blend_9km.png)](viewer/interchange/runs/blend-r4/after/blend_9km.png)<br>`10029D580D154131375105FAA7A08C291D92633F0E8E2F45AB99B3C5FD5783F6` |
| 95.017 km | [![95 km before](viewer/interchange/runs/blend-r4/before/blend_95km.png)](viewer/interchange/runs/blend-r4/before/blend_95km.png)<br>`D7F1B97E6A10F6FBE586FB135C59A79F0B7EC3F07440B4ECFA21013ADB848E35` | [![95 km after](viewer/interchange/runs/blend-r4/after/blend_95km.png)](viewer/interchange/runs/blend-r4/after/blend_95km.png)<br>`FAF3956840F857FACFCEDA3A8EC3F4AFA03F8217BE7884F5A0A033CDCD1C0D17` |
| 288.186 km orbital control | [![orbit before](viewer/interchange/runs/blend-r4/before-orbit/reported_orbit_288km.png)](viewer/interchange/runs/blend-r4/before-orbit/reported_orbit_288km.png)<br>`30CBA123560CCB08E6B07CABCD1032A7EEF1A0825519C86BAF329A148886C26D` | [![orbit after](viewer/interchange/runs/blend-r4/after-orbit/reported_orbit_288km.png)](viewer/interchange/runs/blend-r4/after-orbit/reported_orbit_288km.png)<br>`992E93BBD8A522425D4C1E3E7BDEACDFBF8CA8634AB299FAD3846FD8C7A93A1A` |

Delta is the worst absolute RGB-channel change per pixel, matching the supplied
round-4B metric.

| Comparison | Mean delta | Fraction > 6 | Max |
|---|---:|---:|---:|
| `blend_9km` before -> round 4B | 5.870 | 21.854% | 96 |
| `blend_9km_nowx` before -> round 4B | 6.932 | 23.822% | 112 |
| `blend_95km` before -> round 4B | 1.596 | 5.021% | 20 |
| 288 km before -> round 4B | 2.827 | 8.280% | 79 |
| round 4 -> round 4B, `blend_9km` | 4.480 | 19.032% | 96 |

The 95 km after frame is byte-identical to round 4. The 288 km after frame
moved only mean 0.0058, 0.030% over 6, max 37 from round 4.

## World reel review

`world_reel.py` rendered all 24 poses without `--accept`. No VOID lint fired.
The same three poses as round 4 exceed the change thresholds:

| Pose | Reel delta vs accepted | Explanation |
|---|---:|---|
| `jungle` | mean 3.599; 4.32% > 6 | Same round-4 distant mesh-band climate reconstruction (previously 3.594 / 4.32%); foreground voxels and trees remain unchanged. |
| `orbital_coast` | mean 2.389; 8.27% > 6 | Intended additional categorical grass/sand ownership on land. Accepted-frame water remains stable: mean 0.0165 and 0.075% > 6, essentially identical to round 4's water delta. |
| `orbital_biomes` | mean 2.833; 8.28% > 6 | Same round-4 fractal climate edges and 0.45 orbital contrast (previously 2.829 / 8.26%). Broad continental placement is unchanged. |

`mid_altitude` remains below both thresholds at mean 0.680 and 0.00% over 6.

## Gates

| Gate | Result |
|---|---|
| `cargo build --release --example play` | PASS |
| `cargo test --release --lib` | PASS: 44/44 non-ignored; the separate ignored transect evidence test also passes |
| Seven play suites | PASS: physics, water, lake, flooded caves, invariant survey, regenerated 69-probe auto survey, camera controls |
| Climate comparator occupancy | PASS: 0.25053 / 0.50227 / 0.75195 at requested 0.25 / 0.50 / 0.75 |
| Beach comparator occupancy | PASS: calm full 0.25345 / 0.50748 / 0.74924; calm range 0.25354 / 0.50702 / 0.75015; rugged full/range also within 0.015 |
| `sync_diff.py` | PASS: `sea_calib` 0.0% divergent; mean/p95/bias all zero |
| `world_reel.py` | REVIEW/PASS: 24 rendered, no VOID finding, three flags explained above, no accept |
| `tile_cost` | PASS: final median 0.271 s (0.258, 0.271, 0.262, 0.300, 0.323), budget 0.300 s; unchanged-binary confirmation median 0.268 s |
| Determinism | PASS: two final `blend_9km` renders byte-identical, SHA-256 `10029D580D154131375105FAA7A08C291D92633F0E8E2F45AB99B3C5FD5783F6` |
| Difficulty Lake `colcensus` | exact baseline: `LIP 0`, `EDGE 0`, `SHOAL 30069`, `CAVEP 462` |
| Pond-country `colcensus` | exact baseline: known one-block debt `LIP 88`, `EDGE 0` |
| Full `census --lips` | exact baseline over 221,841,122 samples: `WALL 0`, `JUMP 0`, `SEAJUMP 0`, `LIP 61344`; liquid lake 740, one-block liquid lake 576 |

One immediately post-link, system-contended tile-cost batch measured a 0.318 s
median; two subsequent unchanged-binary medians were 0.268 and 0.271 s. The
reported gate uses the final warmed five-run measurement, consistent with the
round-4 methodology.

No commit was created.
