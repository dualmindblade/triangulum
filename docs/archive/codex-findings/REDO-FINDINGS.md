# A-4/A-5 water redo findings

Branch `sol/water-redo`, based on `adea82f`; world `output/seed42_r8`.
No commit was made.

## Result

Both fixes pass the complete requested gauntlet.

- A-4 no longer constructs the photographed pond rampart. If a pond would
  require more than 3 m of support, the water, dig, bench, fill, and apron are
  rejected as one transaction and the undisturbed natural sample is returned.
- A-5 keeps block occupancy and water classification unchanged, but the
  affected mixed lake/river cohort renders and swims at its common analog lake
  level. The marked voxel top loses its patch lift continuously and hands off
  exactly to the retained mesh water plane at the selected patch radius.
- The other 22 world-reel poses are byte-for-byte identical to the accepted
  baseline. No VOID lint fired.

## A-4: pond ramparts

The support profile is exact through 2 m, fades smoothly to zero by 3 m, and
cannot leave the old hard-cap fence. The transactional gate is limited to the
actual coastal-mask mismatch (`wmask > 0.50`); established inland pond country
continues through its old path byte-for-byte. A rejected mask column can gain
an apron only when a nearby authoritative sample confirms that retained pond
water really exists, so killing water also kills orphaned excavation and banks.

Two unit tests pin this behavior:

- `pond_fill_is_bounded_continuous_and_dies_at_the_cap`
- `unbankable_rampart_pond_reverts_its_whole_transaction`

### Reel flag: `dossier_ponds`

The flag is intentional: mean delta `67.701`, changed fraction `0.8102`.
The accepted frame shows the unsupported blue pond/bench occupying most of the
crop. The new frame shows the natural dry terrain; the rejected pond contributes
no water, carve, bench, fill, or apron. VOID blob is `0 px`.

Before:

![A-4 accepted crop](a4_before_crop.png)

After:

![A-4 fixed crop](a4_after_crop.png)

## A-5: rim water band

The trigger is restricted to unfrozen water where graph-river level exceeds a
finite local lake level by more than 2 m. Integer block occupancy is not
changed. Only presentation and collision use the analog lake target for this
cohort. Marked top vertices move from the lattice ceiling to that target; over
the outer 15% of the patch they continuously shed the normal patch lift. Past
the selected radius, marked voxel fragments are discarded in favor of the
already-retained mesh water plane. Side/foam faces are suppressed only between
marked neighbors sharing the exact same analog plane.

The unit test
`rim_water_handoff_is_flush_without_changing_occupancy` pins the photographed
river block at 125, the adjoining lake block at 122, their shared analog level,
the flush rim top, matching swim physics, and unchanged Difficulty Lake lattice
behavior.

### Reel flag: `dossier_band`

The flag is intentional: mean delta `1.673`, changed fraction `0.0299`.
Only the thin pale lattice side/foam band disappears; the water shape and the
rest of the frame remain unchanged. VOID blob is `0 px`. The reported grid
score (`0.589`) is the reel's informational texture autocorrelation metric, not
a finding; the after crop is a single flat water plane and no per-quad topology
was introduced.

Before:

![A-5 accepted crop](a5_before_crop.png)

After:

![A-5 fixed crop](a5_after_crop.png)

## Gauntlet

### 1. Release library tests

`cargo test --release --lib`: PASS, 33 tests.

### 2. Seven play suites

The release `play`, `colcensus`, and `census` examples were built first. All
seven suites pass:

1. `physics-regressions`
2. `water-regressions`
3. `lake-regressions`
4. `flooded-caves`
5. `invariant-survey`
6. `auto-survey` (69 generated probes)
7. `camera-controls`

The repository's Bash wrapper could not start because WSL is unavailable in
this Windows worktree, so its exact build/generation/suite sequence was run
directly with the same release binary and scripts.

### 3. Mesh/voxel sync diff

`python scripts/sync_diff.py`: PASS. `sea_calib` is `0.0%`, mean `0`, p95 `0`.
All water controls (`lake_shore`, `karst`, and `sea_calib`) match an independently
built untouched `adea82f` baseline exactly. No pose's p95 rose. Divergent-pixel
fractions were equal or lower everywhere; Jungle improved by 0.46 percentage
points and 0.41 mean-delta units. Separate-build streaming coverage produced
only mean-delta noise of `+0.01` at `river_low` and `+0.03` at `groves`; their
p95 did not rise and the full world-reel frames are byte-identical.

### 4. World reel, VOID lint, and determinism

The reel reports exactly two changed poses, both explained above:

| Pose | Mean delta | Fraction > 6 | VOID blob |
|---|---:|---:|---:|
| `dossier_ponds` | 67.701 | 0.8102 | 0 px |
| `dossier_band` | 1.673 | 0.0299 | 0 px |

All other 22 accepted/current PNG pairs have identical SHA-256 hashes and
identical pixels. The only harmless near-black components elsewhere are the
accepted one-pixel components in `coast_beach`, `desert`, and `groves`, far
below the 400-pixel VOID threshold. Two complete consecutive reel renders
produced identical hashes for all 24 frames.

Changed-frame hashes:

| Pose | Accepted SHA-256 | Fixed SHA-256 |
|---|---|---|
| `dossier_band` | `E977E5EA68165C168CD23C56970A53A671EBDEC4F6C2440C5BF979177F30F1D2` | `BD00FCE4CB87CE02461ECB74E1EF70BE0C9C2008CBFD42DDD3042339F7E1CA2E` |
| `dossier_ponds` | `9C730575714BAA16B00E10588BC46F281E2A6CE04E5D2D9130BA385AA6EF4B89` | `D23E6653E0C0EFCA9135EACE93168B778E446E18268E84BF93796A5932534E53` |

### 5. Column census

| Probe | Baseline | Fixed | Interpretation |
|---|---:|---:|---|
| Difficulty Lake `13.346 -4.807` | LIP 0 / EDGE 0 | LIP 0 / EDGE 0 | unchanged |
| Pond dossier `12.199 -44.827` | LIP 336 / EDGE 219 | LIP 88 / EDGE 0 | remaining 88 are all one-block drops |
| Rampart `12.194 -44.858` | LIP 215 / EDGE 136 | LIP 0 / EDGE 0 | photographed structure removed |

The radius-3 site census at the exact rampart fell from 67 WALL sites to 0:
the P-1 cluster of 28 and all 39 nearby sites are gone. At the broader dossier
probe, WALL sites fell from 3 to 1. The survivor at `12.191 -44.807` (2.1 km
away) is the pre-existing stable control outside the gated cohort.

### 6. Global liquid-lip census

| Metric | Untouched `adea82f` | Fixed | Change |
|---|---:|---:|---:|
| Raw LIP findings | 61,360 | 61,344 | -16 |
| LIP sites | 42,050 | 42,035 | -15 |
| Samples | 221,841,122 | 221,841,122 | 0 |
| `LIQUID_LAKE` | 740 | 740 | 0 |
| `LIQUID_LAKE_1_BLOCK` | 576 | 576 | 0 |

WALL, JUMP, and SEAJUMP remain zero. No liquid cohort rises; specifically, the
one-block subtotal stays at 576.

## Additional required checks

- Voxel-off A/B was rendered at both bug poses. The four source frames are
  `rampart_vox.png`, `rampart_mesh.png`, `band_vox.png`, and `band_mesh.png` in
  this directory. All four have VOID blob `0 px`.
- At the band pose, voxel/mesh A/B mean max-channel delta is `0.029934`; only
  `0.00020182` of pixels exceed 6. This is the residual patch geometry, not a
  water-level seam.
- Walk-mode swim check at `13.0128 -4.5462`: `has_water=true`,
  `underwater=true`, and the physics surface is `0.12400894883871079 km`, the
  same analog surface used visually. See `band_swim.json`.
- Difficulty Lake retains its old class, shape, occupancy, and lattice physics.
- Repeat-render byte determinism passes for the complete 24-pose reel.

