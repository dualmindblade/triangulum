# B3: picking across cube seams and corners

Branch: `b3-picking`, based on `54d21ebaf5d395eec1bf772e5a2c58f03ec9549e`.
Changes are intentionally uncommitted for the lead. HEAD matched main;
the round's `git merge main` attempt failed on the linked worktree's
out-of-workspace `ORIG_HEAD.lock`, without changing either ref.

## Diagnosis and change

The suspicion was correct. The old picker used a straight ray in the
eye's tangent lattice, then wrapped the selected cell. Wrapping an address
did not correct the ray's path. Near a three-face corner it could identify
a block beside the actual ray. The outline then wrapped eight corners of
that extrapolated box independently, sometimes folding the box across
several cube faces.

`player/picking.rs` now intersects the straight world ray with the six
cube-boundary planes, processes each face interval in its own coordinates,
and solves integer-coordinate crossings with the exact inverse along the
world ray. It also splits at radial closest approach and subdivides spans
where a nearly tangent lattice coordinate may turn. Root guesses come from
chords; accepted crossings are evaluated on the world ray. Reach is
inclusive with a 1e-8-block forward-limit tolerance at its endpoint.

`Hit.face`, `Hit.cell` and `Hit.normal` now all belong to the hit block's
own lattice. `place_cell()` takes one topological neighbour of that block;
the session resolves that neighbour before checking the player's body.
Bucket picking uses the same traversal and continues to skip the eye's
own water cell.

The placement guard previously returned false for a neighbouring cell
whose centre belonged to another cube face. It now transforms that cell's
eight real corners into the body's local frame and uses separating axes
to check overlap. Walking movement itself is unchanged.

All outline callers now pass the canonical `BlockPos`. The outline uses
the LOD-0 mesh/shader curvature grid (including f32 interpolation), so
neither an out-of-face box nor a tangent-space box is drawn.

## Independent reference and precision

`picking_tests.rs` constructs edited stone and water cells on and beside
seeded random world rays. Its oracle only samples `eye + direction * t`,
calls `Planet::lattice_of_dir`, and floors the exact radial coordinate.
It does not call `Frame`, `wrap_cell`, the picker, or its crossing solver.

There are 128 rays per site, all eight corners, all twelve edges (random
positions along their lengths), and six ordinary face-interior sites.
Both one-metre and half-metre blocks and both solid/bucket queries are
tested: 6,656 rays / 13,312 comparisons. The frozen pre-B3 picker is kept
only in `picking_legacy_test.rs` for a reproducible before measurement.

The initial sample spacing is 1 mm for metre blocks, 0.5 mm for half-metre
blocks. A cell traversed for at least that distance cannot be skipped;
the first observed point trails entry by at most one step. Mismatching
identities are rechecked at 1/32 spacing (31.25 or 15.625 micrometres).
The test also checks the exact inverse immediately before and after every
nonzero hit: after entry it must name the hit, before entry it must name
the resolved placement neighbour. Thus the tolerance cannot excuse an
off-ray target or an unrelated placement cell.

The fixed picker has **0 / 13,312 disagreements after refinement**. The
initial march missed four comparisons involving three short intervals:
0.422017 mm on an edge, 0.255052 mm on ordinary ground (both query modes),
and 0.388987 mm at a corner with half-metre blocks. All were found by the
1/32-step oracle, and the exact entry/placement checks passed. The suite
contains 100 rays visiting three cube faces and 2,309 hits on another face
than the eye. This is an empirical bound at the stated resolution, not a
claim that a finite sample oracle proves every possible grazing ray.

| Sites | Comparisons | Before, confirmed wrong cell | After, confirmed wrong cell |
|---|---:|---:|---:|
| 8 corners | 4,096 | 712 (17.3828%) | 0 |
| 12 edges, varying position along each | 6,144 | 476 (7.7474%) | 0 |
| Ordinary ground on 6 faces | 3,072 | 0 (0%) | 0 |
| **Total** | **13,312** | **1,188 (8.9243%)** | **0 (0%)** |

Before refinement the old picker had 1,191 wrong-cell comparisons; three
were reference sampling misses. In addition to the confirmed wrong cells,
623 old-picker queries named the right cell but reported a distance outside
the coarse oracle's step tolerance. The before reference test fails as
expected. The old overlap guard also fails the independent body sample
test; the corrected guard refuses all 80 sampled overlaps on other faces.

Timing is **provisional**, with concurrent work on this machine. Each row
below uses 83,200 queries, split between solid and bucket paths, over the
same seeded corner/edge/ground scenes. Values are wall-clock microseconds;
the sparse edited-cell `Voxels` isolates traversal from terrain generation.

| Query set | Picker | Mean us | Median us | p95 us | p99 us |
|---|---|---:|---:|---:|---:|
| Mixed hits/misses | Before | 5.800 | 2.900 | 4.000 | 5.700 |
| Mixed hits/misses | After | 77.341 | 58.700 | 89.900 | 178.200 |
| Empty, full 6 m reach | Before | 3.119 | 2.900 | 3.300 | 4.000 |
| Empty, full 6 m reach | After | 173.186 | 78.100 | 111.000 | 240.000 |

The full-reach median is 0.078 ms; its mean is 0.173 ms, about 1.0% of a
16.67 ms frame. This is more work than the old tangent DDA but still a
small per-frame cost. Large wall-time outliers occurred on both versions
(up to 35 ms before and 109 ms after); these are not idle-machine bounds.
The required flight's frame benchmark was also contended (27 fps, GPU
3.31 ms), so this run does **not** verify an idle 60-fps frame-rate target.

Additional tests cover radial and lateral grazing entries with matching
endpoint columns/layers, exact cube seams and corners, the reach limit,
water skipped in the eye's cell, solids behind water, all-face body
overlap, and outline vertices against the actual mesher's uploaded grid.

## Visual reproduction

`game/scripts/b3-corner.play` uses the exact latitude, longitude, feet
altitude, heading and pitch from replay steps 800 and 1300. It uses fresh
terrain and a fixed noon sky; it does not import the recording's 35 edited
chunks. It takes the same shots with the frozen `945d544` executable and
the changed build. The frozen run visibly reproduces the folded outline
at step 800 and floating outline at step 1300. Before/after frames are
game captures only in `B3-picking-evidence/`.

At step 800 the frozen build selects `(1, 2, 131070, 523)`, while the real
ray hits `(4, 131069, 131071, 523)`. The placed stone belongs at
`(4, 131069, 131071, 524)`: the fixed frame shows its outline coinciding
with the stone's visible edges. The script checks that cell, removes it,
and repeats with a full water cell using the bucket.

**Step 1300 needs a distinction.** Both builds and the recorded JSON name
`(2, 131068, 131070, 523)`. Their distances differ by only 0.0865 mm
(4.10607850 m before, 4.10616497 m after). This view does not demonstrate
the extrapolated-ray failure. Its full box outline still looks suspended
because `draw_hud` draws all twelve edges over the image, including edges
behind terrain and foreground stone. B3 corrects the block geometry and
the target, but does not change that existing visibility policy. The
step-1300 placed-stone frame confirms the box fits the actual stone.

At the exact vertex the script looks down at cells on faces 4, 1 and 2
while the player belongs to face 2. Each attempted placement would enter
the body, and each is refused. Block checks verify the cells stay air.
The frozen and local `blocks.json`, `look.json` and `world.json` were
byte-identical for these runs.

The final script has **52 checks**: frozen build **12 failed**, fixed
build **52 passed**. Both were run with this worktree's explicit world,
block and look files and separate scratch saves. I inspected the original
recording frames, both scripted poses before and after, and the placed
stone and bucket frames.

| View | Before | After |
|---|---|---|
| Step 800 target | [folded box](B3-picking-evidence/before_step800.png) | [correct cell](B3-picking-evidence/after_step800.png) |
| Step 800 placement | [wrong placement](B3-picking-evidence/before_step800_placed.png) | [stone and matching outline](B3-picking-evidence/after_step800_placed.png) |
| Step 1300 target | [target](B3-picking-evidence/before_step1300.png) | [same correct cell](B3-picking-evidence/after_step1300.png) |
| Step 1300 placement | [stone](B3-picking-evidence/before_step1300_placed.png) | [stone and matching outline](B3-picking-evidence/after_step1300_placed.png) |

Also included: [bucket](B3-picking-evidence/after_bucket.png),
[body guard](B3-picking-evidence/after_body_guard.png), broken-block frames,
and the two supplied recording frames for context. The 14 unmodified game
PNGs total **2,429,037 bytes (2.32 MiB)**; no desktop captures are included.

## Audit: related assumptions left alone

- `Player::step_walk`, `sweep`, `sweep_horizontal`, `free_box` and
  `box_is_free` still use a local rectangular lattice and wrapped cell
  lookups for a body that straddles an edge. The transition code sweeps
  the remaining displacement in a second face frame, but a second edge
  in one step uses a tangent displacement plus an end-position box check.
  This is still an approximation at three-face corners. No collision
  solver or movement rule was changed in B3.
- `Frame::from_lattice` is a local differential frame. Its derivatives
  are clamped to its face. It remains appropriate for local directions;
  it is no longer used to extrapolate a picking ray.
- At an exact vertex, `Frame::feet_cell` / the state sidecar can still
  report i or j equal to N and rely on wrapping for lookups. Picking now
  returns canonical cells even there. Changing the collision/state helper
  is separate from B3 and was left alone.
- Harness `aim`, `target rel` and `pan ... aim` already construct a world
  point with `Planet::pos` (which wraps), then aim the real world direction
  at it. `lookaxis` and its pan variant use a local tangent, which is a
  direction, not a promise to follow a wrapped index row. Their behaviour
  needed no change; the command documentation now says this explicitly.
- Session relative-cell queries use topological adjacency for named
  offsets, not a world-ray approximation. Ground placement and water
  sampling use the current radial column. These were left alone.
- Liquid picking still selects occupied cells, including partial water;
  it does not intersect the animated/sloped water surface. The selection
  outline remains the whole selected voxel. Water surface behaviour is B1.

## Versions and scope

No save, generator, simulation-rule or replay-format version should be
bumped for B3: terrain, stored cells, simulation rules and the file schema
are unchanged. Old corner recordings can diverge at clicks because their
targets have been corrected; preserve those recordings and use their
original build for exact playback. A separate input/interaction rules
version could be introduced in a future replay-compatibility task if
automatic refusal of old interaction behaviour is wanted.

The original 1,859-step recording was run headless in this worktree's
scratch save. It reports the first world mismatch at checkpoint step 300
(5.00 seconds, one edited chunk differs), matches 4 of 31 checkpoints,
and continues to the end. Its source file and the main checkout's saves
were not modified.

No changes to `mesh.rs`, `worldgen/` or `sim/`. The changes in `ui.rs` and
`app.rs` are limited to the outline. No edit to STATUS, PLAN or DESIGN is
proposed beyond marking B3's picking correction complete after review;
the existing corner collision caveat should remain.

## Verification and limitations

| Check | Result |
|---|---|
| `cargo test --release -j 2 -- --nocapture --test-threads=2` | 173 passed, 0 failed, 15 ignored; exit 0 |
| `scripts/m1-done.play` | 94 checks passed; exit 0 |
| `scripts/tunnel-wall-reload.play` | 18 checks passed; exit 0 |
| `scripts/edge-corner-flight.play` | 17 checks passed; zero coverage violations; exit 0 |
| `scripts/m2b-water.play` (additional bucket regression) | 97 checks passed; exit 0 |
| `scripts/b3-corner.play` | 52 checks passed; exit 0; frozen build fails 12 |
| Original recording, current build | Expected first changed world checkpoint at step 300; completed 1,859 steps |
| `git diff --check` | Clean |

The earlier M1 edge-picking unit test aimed along +i in the eye's frame
and assumed that reached the neighbour's wrapped index row. The real ray
misses that cell. It now aims at the neighbour's actual world centre and
continues to check that breaking names the canonical neighbour. The
initial full-suite failure was this stale aiming assumption; the final
full suite passes.

Logs and JSON sidecars remain in ignored `game/out/`: `b3-tests-final.log`,
`b3-reference-before-final.log`, `b3-cost-final.log`, `b3-body-before.log`,
`b3-before/`, `b3-after/`, `b3-regression/` and `b3-replay-after/`.
On a normal SDK-equipped machine, reproduce from `game/` with:

```powershell
cargo test --release
cargo test --release b3_pick_cost -- --ignored --nocapture
$env:B3_BEFORE = '1'
cargo test --release b3_world_ray_reference -- --nocapture  # expected failure
cargo test --release b3_body_overlap -- --nocapture         # expected failure
Remove-Item Env:B3_BEFORE
cargo build --release
target/release/triangulum.exe --bake C:/code/triangulum/game/assets/world/seed42 --play scripts/b3-corner.play --out out/b3-after
```

For the frozen comparison, replace the executable in the last command
with the `baseline-945d544/triangulum.exe` path from the round page, use
`--out out/b3-before --save saves/_play/b3-before --reset-save`, and pass
`--world assets/world.json --blocks assets/blocks.json --look-file assets/look.json`.
The same script intentionally fails its corrected-target assertions there.

All execution was headless. Real mouse input, walking and placing in a
visible window, and an idle-machine performance run were not verified.
All timings are provisional because other missions share the machine.

The Windows SDK library directory was missing in this environment. To
build, temporary import libraries were generated from the installed
Windows system DLL exports under ignored `game/out/sdk-lib/`, with the
installed MSVC linker and runtime libraries. No compiler flags, dependency
versions, game sources or system installation were changed for that
workaround. Build artifacts are in the ordinary `game/target/` on E:.
After the successful full suite, the MSVC linker and runtime library
directory also disappeared from the machine. A final optional fixture
experiment could not link and was reverted; delivered Rust code is the
version exercised by the passing full suite. Rebuilding now requires
restoring a complete MSVC/Windows SDK installation.

## Changed files

- `game/src/player/picking.rs` (new): world-ray traversal.
- `game/src/player.rs`: canonical hit contract, placement overlap, corrected
  old edge-test aim and test-module wiring.
- `game/src/session.rs`: check the canonical placement cell against the body.
- `game/src/ui.rs`: canonical outline and the LOD-0 mesh-grid corners only.
- `game/src/app.rs`, `game/src/play.rs`, `game/src/play/video.rs`: outline
  callers; the play command documentation also clarifies local tangents.
- `game/src/picking_tests.rs` and `game/src/picking_legacy_test.rs` (new):
  independent oracle, regressions, timing, and preserved before algorithms.
- `game/scripts/b3-corner.play` (new) and `game/README.md`.
- `docs/missions/B3-picking-findings.md` and the 14 PNGs in
  `docs/missions/B3-picking-evidence/`.
