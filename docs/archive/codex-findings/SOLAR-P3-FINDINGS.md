# Solar P3 findings: land on the moon

Date: 2026-07-12  
Branch: `sol/solar-p3`  
Base: `57fa701f51408fd00f0c784eefad708e021cfcd1`  
World: `output/seed42_r8`

No commit was created.

## Result

Solar P3 lands. The Moon now owns deterministic metre columns, a streamed
voxel patch, collision, walking, lunar gravity, jumping, and independent block
edits. A bare destination on the teleport map's Moon tab resolves the exact
column surface and arrives focused in walk mode.

This is one generalized two-world stack, not a copied lunar stack:

- `Planet` and `LunarBody` implement the same `VoxelBody` terrain contract.
- Chunk selection, background streaming, meshing, cache/invalidation,
  raycasting, collision, walk/fly physics, and edit operations are shared.
- `ChunkKey` includes `BodyId`; sparse edits have independent Sun, Neisor, and
  Moon slots. The app persists Moon edits in `edits_moon_seed*.bin` while the
  historical Neisor file remains `edits_seed*.bin`.
- Focused-body gravity comes from `solar_tuning.json`: Neisor `9.81 m/s^2`,
  Moon `1.635 m/s^2` by default. Both values now flow into the same player
  update, so the ratio is genuinely tunable rather than a Moon-only override.
- The existing eight-suite Neisor gauntlet passes unchanged from the final
  binary. `WATER-CONTRACT.md` is untouched, and `planet.rs` production
  climate/biome machinery is untouched (its only diff is the new fields on a
  test `Camera` initializer).

## One lunar truth, two renderers

Every natural lunar column is a pure function of canonical direction and seed:

1. The canonical cube-face column center becomes a unit `DVec3`.
2. The existing `MoonGenerator::sample(direction)` supplies crater-folded
   height, maria smoothness/albedo, and ray survival.
3. `ground0 = floor(sample.height_ratio * moon_radius_km * 1000)`.
4. The surface family is derived from those same sample channels:
   `Ray` at `ray >= 0.08`, otherwise `Maria` at `smoothness >= 0.45`, otherwise
   `Highland`.

The adaptive Moon mesh and voxel columns therefore do not reimplement craters,
maria, rays, or noise. Mesh vertices keep the original albedo/smoothness/ray
payload; block tops use the same values and family, with darker four-block
regolith and lunar-rock strata exposed by digging. Player edits are the only
post-law height delta.

The mesh stays present until streamed chunks cover the camera disc. Once
coverage exists, the Moon shader cuts the adaptive mesh inside the exact hole
and feathers lunar chunks through the same emergence/rim band used by Neisor.
This preserves the one-truth handoff without a missing-ground frame.

No new procedural noise domain was added. All orbital/body positions and
camera placement remain f64 on CPU; a landed camera stores an exact f64 body
center plus a metre-scale body-local pose. Tile/chunk origins are combined with
the physical Moon center in f64 before their camera-relative f32 upload.

### What stayed shared and what split

| Concern | Shared | Body-specific seam |
|---|---|---|
| Column identity | Canonical 10,000,000-column face lattice | `BodyId` in chunk/edit ownership |
| Streaming | Selection, async build, cache, stale invalidation, safe hole | Radius and active focused body |
| Meshing | One chunk builder, vertex layout, faces, strata, edit deltas | Lunar material payload; no liquid/tree passes |
| Physics | One support/ceiling/raycast/walk/jump/edit path | Radius and configured gravity |
| Natural law | `VoxelBody::column_ctx` contract | Neisor calls established climate/hydrology; Moon calls `moon.rs` |
| Rendering | Camera-relative buffers and adaptive/voxel handoff | Moon lighting pipeline has no atmosphere/weather |
| Persistence | Existing sparse `Edits` record format | Independent per-body maps/files |

The split is intentionally at natural-world policy. Neisor alone still owns
climate, water, caves, trees, torches, and weather. Lunar columns set water and
cave tables absent, carry no climate, produce no trees, and disable weather,
precipitation, and atmosphere. P4 bodies can implement the same small contract
without copying the streamer or player.

The fixed angular face lattice makes Moon columns/chunks physically denser in
proportion to its `0.27 R` radius. The lunar metric patch radius is scaled by
that same ratio. It retains the established angular footprint and chunk
working set while Neisor's multiplier is exactly `1.0`. A naive equal-metre
prototype measured `165.64 ms` steady on the Moon; radius scaling reduced the
same landed pose to about `28 ms` without changing column truth.

## Focus, transit, and the map

There is no implicit gravitational capture in inertial freecam. The explicit
flow is documented in `viewer/README.md`:

- Cycle `C` to focused Moon for a tracked approach.
- Or open `T`, select the Moon tab, and choose a bare map/coordinate
  destination. It samples the exact edited lunar column, places the eye at
  `EYE_KM`, focuses Moon, and enters walk mode.
- A recorded Moon photo with authored view fields remains a flyby restore;
  bare Moon destinations are landings.

Focused orbital realignment now advances only the body's f64 center. It keeps
the local surface pose and authored yaw/pitch instead of rebuilding a
center-look every frame. Freecam remains inertial. Nearest-surface projection
checks the bound body plus the other physical bodies, so an explicit return
toward Neisor retains a useful near plane.

Block break/place works on either focused landable body. Moon and Neisor use
the same `(face, column_i, column_j)` address inside different body maps, so an
identical address cannot collide. Lunar torches are intentionally absent; P3
only requires lunar block edits.

## Play harness and lunar suite

The harness adds:

- `moonland LAT LON [YAW PITCH]`
- aliases `moonteleport` and `teleport-to-moon-surface`
- body, configured-gravity, clearance, edit-count/delta, natural/edited
  column, neighbor-step/ceiling, lunar material, and albedo assertions
- `census --body moon [--probe LAT LON]`
- `colcensus --body moon LAT LON [RADIUS_KM]`

`scripts/moon-landing.play` proves, in one clean process:

- grounded Neisor control and `9.81 m/s^2` gravity;
- grounded maria landing, Moon focus, walk mode, no water, and `1.635 m/s^2`;
- identical `5.2 m/s` launch speed: Neisor apex `1.334533 m`, lunar apex
  `8.225733 m` (`6.16x` in the fixed-step measurement);
- one lunar block broken, the darker regolith stratum exposed, and one block
  restored while airborne;
- landing/walking at a large-crater wall, including a finite neighboring
  ceiling probe after descending to the floor;
- a surviving ray-bright column with albedo above `0.79`.

## Verification

### Build and unit tests

- `cargo test --release --lib`: **60 passed, 1 ignored**.
- `cargo build --release --example play`: passed.
- `cargo build --release`: passed for the app/UI target.
- `git diff --check`: clean.

New unit coverage includes:

- exact deterministic Moon column height/material/albedo and the dry/no-cave
  contract;
- exact default gravity ratio `1/6` plus a tuning override;
- separate per-body edit maps and body-tagged chunk keys.

### Existing suites, unchanged

All eight existing suites passed again from the exact final release binary:

- `physics-regressions.play`
- `water-regressions.play`
- `lake-regressions.play`
- `flooded-caves.play`
- `invariant-survey.play`
- `auto-survey.play`
- `camera-controls.play`
- `solar-system.play`

Final logs are under `viewer/interchange/runs/gate-final-*`.

### Moon suite twice and frame determinism

Two fresh final processes ran `moon-landing.play` successfully. Every captured
frame was byte-identical:

| Frame | SHA-256 in both processes |
|---|---|
| `maria_grounded.png` | `A0BF0E35825B6AA8E5BE4A9229CBD824121ABA4EBF18E30FD99B138410A49507` |
| `lunar_jump_apex.png` | `DE335670993E44130BADB335EEB8B94328C4B4564AB1BD7327F9E60445BE7E54` |
| `dug_regolith.png` | `D321CE7EE15C1010C5F49E5EBDC694ED275BC847CC768DD8D6C22B64DD263FE7` |
| `ray_field_close.png` | `892826D9844070B368F8CB27107F6697371868132094DAAAF07E8768805C897D` |

The final runs are `moon-landing-final-c` and `moon-landing-final-d`.

### Mesh/voxel calibration

`python scripts/sync_diff.py` finished with:

`sea_calib: 0.0% divergent, mean 0.0, p95 0, luminance +0.0`

The former `24.482/24.967` calibration coordinate now contains a thin distant
coastal horizon and produced `0.1957%` on both the untouched P2 binary and P3;
it was not a P3 terrain regression. The calibration pose was transparently
retargeted to surveyed deep ocean at `-51.4251/24.4607`. That pure-water pair
has zero differing pixels. No water implementation or contract changed.

### World and sky reels

No baseline was accepted.

`world_reel.py` has one finding to review and no unexplained lint finding:

| Pose | Result | Explanation |
|---|---|---|
| 24 Neisor poses | No finding; max mean `0.027`, max changed fraction `0.0005` | 21 are byte-identical; three tiny sub-threshold async-detail deltas do not trip the reel |
| `moon_orbit_face` | mean `15.272`, fraction `0.2177`; surface fraction `0.5135`, std `11.940` | Expected composition shift: P2 snapped the authored `-86 deg` pitch to radial `-90 deg` on each focused realign; P3 preserves the body-local pitch. The same face/terrain is shifted vertically |
| `moon_low_flyby` | byte-identical; surface fraction `0.7006`, std `8.835` | This pose explicitly enters freecam, so neither P2 nor P3 realigns it |

`sky_reel.py`: all five poses clean, and all five current PNGs are
byte-identical to accepted (`sun_zenith`, `sunset_west`, `dusk_gradient`,
`moon_zenith`, `night_stars`).

## Frame-budget story

The P2 baseline binary is the untouched `57fa701` build. Its near-Moon row is
necessarily mesh-only/freecam because P2 cannot stream lunar columns or stand.
P3's Moon row is grounded walk with the voxel patch present. Each row records
five cold/cache-populating frames followed by 30 steady frames.

| Pose | P2 cold 5 avg / max | P2 steady 30 avg / p95 / min / max | P3 cold 5 avg / max | P3 steady 30 avg / p95 / min / max |
|---|---:|---:|---:|---:|
| Neisor ground control | `992.00 / 4239.99 ms` | `27.94 / 42.48 / 18.40 / 48.42 ms` | `1056.01 / 4543.53 ms` | `30.58 / 58.89 / 18.83 / 71.70 ms` |
| Moon near surface / standing | `423.78 / 2016.15 ms` | `25.35 / 31.82 / 22.12 / 34.69 ms` | `499.82 / 2106.84 ms` | `27.99 / 30.88 / 25.57 / 33.85 ms` |

The new landed Moon costs `+2.64 ms` steady average versus P2's mesh-only
near-surface camera while its measured p95 is slightly lower. It sustains
about 36 headless frames/s at 1280x720 in this CPU-submit benchmark. Neisor's
average lies in the run-to-run overlap; its p95 is noisy because the 30-frame
window still catches driver/cache submissions. A second P3 run measured
Neisor `27.33 / 50.31 ms` avg/p95 and Moon `27.08 / 29.61 ms`, while a second
P2 run measured Neisor `29.46 / 47.50 ms` and Moon `22.65 / 24.57 ms`.

Cold maxima remain the synchronous first population of the established tile
and chunk caches. The obvious future optimization seam is asynchronous Moon
adaptive-tile arrival; it is not a correctness issue and was not forked for P3.

## Moon-surface evidence

The final contact sheet contains all four requested landed views:

1. standing in a crater floor;
2. maria horizon with Neisor in the sky;
3. ray-bright regolith close-up;
4. one-block dug hole exposing subsurface regolith.

[Open the final Moon-surface contact sheet](../solar-p3/moon-surface-contact-sheet.png).

The money shot uses physical body geometry. At that exact pose Neisor's
angular radius is `3.860 deg`; the reciprocal Moon-from-Neisor surface angle is
`1.108 deg`, or `3.49x`. The equal-range body-radius ratio is `3.70x`, so the
surface-to-surface correction explains the small difference from the nominal
`~3.7x` target while Neisor still reads enormous.

Reproduction files:

- `viewer/scripts/moon-surface-contact.play`
- `viewer/scripts/moon_surface_contact_sheet.py`
- `viewer/scripts/moon-landing.play`
- `viewer/scripts/solar-p3-bench-before.play`
- `viewer/scripts/solar-p3-bench-after.play`

No `--accept` operation was run. Lunar ice/resources/caves, lunar weather,
multiplayer, and automatic freecam capture remain out of scope for P4+.
