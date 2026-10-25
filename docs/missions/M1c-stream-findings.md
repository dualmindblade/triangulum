# M1c-stream: streaming you can trust, finer distant detail (Fable, 2026-10-25)

Branch `worktree-agent-a39e5a1dc735ceceb`, crate `game/`, merged with main
through landforms v2 (604854c). Evidence in `docs/missions/M1c-stream-evidence/`
(downscaled) and, full size, under
`C:\code\triangulum\.claude\worktrees\agent-a39e5a1dc735ceceb\game\out\`
(gitignored, this machine only). Scripts: `game/scripts/edge-corner-flight.play`
(new), `game/scripts/water-lod-seam.play` (new), `game/scripts/m1-done.play`.

## What was built

| module | what |
|---|---|
| `stream` | The per-frame rebuild of the desired set is gone. `Coverage` is a persistent tree with one node per chunk that exists (roots at the coarsest LOD, children down to wherever the camera pulled detail) and, per root, a *cut* of `drawn` nodes: what the renderer shows. The cut is complete and never overlaps. It moves **down** when a drawn node wants finer detail: its children are dispatched while it keeps drawing, and when every child is resident (or beyond the horizon) the children become drawn and it stops, in one step. It moves **up** when an interior node's distance says the detail is no longer wanted: it gets a mesh of its own first, and only when that mesh is resident does it become drawn and the subtree go. A parent keeps its mesh while a child it switched to is unloaded behind the horizon, and draws again if that child comes into view before loading, so a horizon crossing never leaves a hole (review #12). Jobs carry node slot, epoch and revision: a result for a dead node drops, a result from an older revision regenerates (edits), and nothing resurrects a freed chunk. Splits are spread over frames by a time budget. |
| `stream` (horizon) | The occluder is `Planet::bottom_radius()`, the one sphere that stays opaque whatever the terrain or a player's digging does (review #8). It hides less than the old land-minimum sphere (at the ground, roughly beyond 9.6 km instead of 3.3 km), which the tree absorbs: hidden nodes are never dispatched and count as ready. |
| `stream` (ranges) | Footprint height ranges cost hundreds of microseconds each with landforms (the coordinator's 80 µs was an idle-machine figure; 160–450 µs here). A node that wants to split asks for its children's footprint ranges as worker jobs and keeps drawing until they arrive; the split itself (classification, bounds, allocation) then costs microseconds. Ranges live in a bounded oldest-first cache (64 k). The tree has no other cache: a node owns its bounds and dies with them. Worker timing history is recorded only when the bench enables it (review #15). |
| `stream` (classify) | Whether a chunk exists is worldgen's call: `Generator::classify_in(key, range) == Mixed` through a closure, so Air and Enclosed chunks are never nodes (item 4). They are still materialised through `World::generate` when a player digs into or stands on them (physics reads `World::natural`, which is `generate`). Tests use the plain band test. |
| `grid` | `ChunkKey` and `FootprintKey` carry a `body` id (Neisor = 0), `ChunkKey::neisor(...)` for literals. `Planet::chunk_bounds`: a sphere containing every curvature-grid corner at both radial extremes (review #9; the old `half·√3·1.05` was 30.1 m against a 33.8 m corner next to a cube corner), clipped to the world top. `Planet::extended_coords`: the inverse of one `wrap` step, so a neighbouring face's lattice point can be written past this face's side. |
| `mesh` | Bounds from `chunk_bounds` (the renderer's frustum sphere was the same too-small one). The grid record carries `r0`, the lattice floor's radius, in `off[0].w` (`GRID_R0_INDEX`) for the exact-radius vertex correction (review #11, below). |
| `world` | The edit store indexes edited chunks under every ancestor (LODs 1–15), so a coarse chunk's overlay costs the edits near it, not every edit on the planet (M1b's "spatial index" leftover). `World::edited_near` collects them for a chunk, its 26 neighbours, and, on a face edge, the neighbouring faces' chunks along the padding strip. The coarse overlay maps edits across a face edge into the padding (the M1b gap). The physics cache evicts oldest-first, one chunk at a time, instead of clearing (review #15). |
| `player` | Ctrl on the ground sneaks: 1.3 m/s, and a horizontal move is shortened by bisection so the box keeps something solid under it; like Minecraft the box may overhang until its trailing edge is at the block's edge. In water Ctrl still dives, in fly mode it still descends. The help texts and the README say so. |
| `engine` | The `landform` dial goes from `WorldConfig` through `Engine::build`/`rebuild` to `Generator::new` explicitly; the process-wide default in worldgen and its call in `main.rs` are gone. |
| `play` | `travel SECONDS [KEYS]`: moves with the keys held, streams and renders every step, and checks the cover after each frame (overlaps, visible drawn chunks without a mesh, uncovered leaves), reporting the worst counts, the CPU times and the tree step's phases. `speed M_PER_S`. |

Worldgen lines touched (mechanical, the body id and the landform argument):
`worldgen/tests.rs` lines 20, 32, 127, 232–234, 257, 274, 481; in
`worldgen/mod.rs` the four `ChunkKey` test literals that existed before the
landforms merge (519–521, 544 on the M1b main) and, after it, `Generator::new`
(the `landform: f64` parameter) and the removal of `DEFAULT_LANDFORM_BITS`,
`set_default_landform` and `default_landform` (lines 42–55 of 604854c).

### Review findings folded in

- **#8** occluder = world bottom. Test: the trench floor at vertical scale
  1/8 (H 4096, sea 1536; floor at −1375 m, world bottom −1536 m), seen from
  the sea surface 2 km away, is never hidden, nor is any chunk in its column
  down to the bottom. (With the M1a generator the land-minimum occluder sat
  1167 m below sea and hid it: reproduced before landforms v2 widened the
  relief margin; the test now prints that occluder instead of asserting on it.)
- **#9** bounds from the real grid vertices at both radial extremes. Test:
  every one of the 25×2 grid corners of 208 chunks (eight chosen next to
  cube corners and edges, 200 pseudo-random over every face and LOD) lies
  inside the sphere, smallest slack 0.56 m, and the sphere is never looser
  than 2 % + 1 m.
- **#11** water cracks between LODs. The chord sags `r0 (1 − |m|)` below
  the sphere mid-cell: 23.36 m at the LOD-8/LOD-9 boundary (the review's
  23.21 m plus landforms' different `r0`). The mesher now supplies `r0`; the
  fix itself is four shader lines (below), which belong to Astra.
- **#12** the coverage tree, above.
- **#15** no unbounded caches: footprint ranges bounded (oldest-first),
  geometry cache gone (nodes own their bounds), physics cache evicts
  one at a time, timing history bench-only.

### The shader change for Astra (not committed; `M1c-stream-evidence/shader-exact-radius.patch`)

In `vertex()`, after `off` and `up` are interpolated:

```wgsl
let len = length(up);
var sag = 0.0;
if (d.voxel >= 8.0) { sag = grids[d.slot].off[0].w * (1.0 - len); }
let world = d.origin + off + (up / len) * (lp.z * d.voxel + sag);
```

`up` stays as it was for the normals. The three finest LODs skip it: their
sag is under 2 mm and f32 noise in `1 − |m|` (about 9 mm) would show from a
metre away. Measured with the shader's transform emulated in f64 on the real
f32 grid records (`mesh::tests::water_surfaces_meet_across_lod_boundaries_with_the_radius_correction`):
the LOD-8/LOD-9 sea-surface radial mismatch goes from **23.36 m to 2.8 mm**,
with a 2.89 m lateral residue (the lattice line against the great-circle
chord; sub-pixel at the 3 km and more where LOD 9 is drawn). Both chunks'
sea-level vertices land on the sea-level sphere to 5 cm.

## Evidence

### Tests (`cargo test --release`: 70 passed, 1 ignored [Opus's])

New this mission: the coverage-tree invariants under randomized camera paths
(six seeds, 1500 frames each, random walks on a 2.8 km test planet from 2 m
to 2 km with teleports and altitude jumps, jobs completing 1–6 frames late in
random order, a random edit every tenth frame, odd seeds with inline footprint
ranges and even seeds with ranges a frame late; after every step and every
batch of results: no overlapping draws, no visible drawn node without a mesh,
nothing uncovered under a root that has drawn, every fake GPU mesh freed
exactly once; then the camera stops, the jobs drain, and the cover is
complete, settled and at exactly the LOD the policy asks for, with the live
GPU meshes equal to the resident nodes; per seed 3–6.3 k nodes at peak,
~2 100 switches, 700–1 660 merges); an edited Enclosed chunk (a chunk the
band test excludes gets "dug": it appears under its split ancestor, is drawn
once resident, refines down to LOD 0, and never counts as a violation on the
way); atomic switches (half the children
arriving changes nothing, the rest switch the cut in one step and free the
parent; moving away keeps the children drawn until the parent's mesh is
resident, then deletes them; a parent retains its mesh for a hidden unloaded
child and draws again when that child comes into view, then switches down
once it arrives); the occluder against the trench; bounds containment near a
cube corner; `extended_coords` inverting `wrap` at every side of every face;
the ancestor index; edits crossing a face edge into coarse padding at LODs
1–4 (an edit in face 0's last column shows in face 2's padding voxel and
invalidates face 2's chunks) plus one-at-a-time cache eviction; the water
seam numbers above; sneaking (walking off a four-column platform falls,
sneaking stops with the trailing edge on the edge, sideways and backwards
too, releasing Ctrl walks off, Ctrl in water still dives).

### Scripts

`m1-done.play` (the terrain-independent rewrite from main): **94 asserts, 0
failed** on the merged branch; `M1c-stream-evidence/m1-done.log`.

`edge-corner-flight.play` (**17 asserts, 0 failed**; `edge-corner-flight.log`,
`flight_*.jpg`): fly mode at 1 km/s, every frame streamed, rendered and the
cover checked.

| leg | frames | faces | cover violations | nodes max | main thread avg / p99 / max ms |
|---|---|---|---|---|---|
| lat 0, lon 30 → east 30 km across the +X/+Y edge at 3 km | 1800 | 0, 2 | 0 (overlaps 0, gaps 0, uncovered 0) | 3 212 | 0.37 / 1.49 / 4.56 |
| lat 20, lon 44.5 → north 50 km past the (+X,+Y,+Z) corner at 3 km | 3000 | 0, 4 | 0 | 4 976 | 0.43 / 1.21 / 7.04 |
| lon 46 → west 8 km back across the edge at 250 m, 400 m/s | 1200 | 0, 2 | 0 | 8 249 | 0.59 / 2.18 / 9.87 |

The maxima are provisional (below): the phase timers that produced them do
not add up as work (a 2.4 ms "counts" loop over 3 200 nodes, a single split
taking 5 ms) while two other missions were building and benching on the
machine; the averages and p99s are the engine's.

### Bench (PROVISIONAL: Opus and Astra shared the machine)

`cargo run --release -- --bench --lod-factor F`, RTX 2060, Ryzen 7 2700X,
16 rayon threads, 1920×1080 offscreen, Vulkan, the landforms v2 generator.
Full tables in `M1c-stream-evidence/bench-f5.md`, `bench-f10.md`,
`bench-f20.md`, `bench-f5-scale8.md`. "CPU" is the main thread's streaming
update plus the draw encoding; "tree" and "lists" are the streaming update's
own phases from the bench log, at the settled pose.

Static poses, 120 frames each after settling (ms averages):

| factor | pose | GPU | CPU (p95) | tree / lists | draws | resident / nodes | retained | mesh VRAM MB |
|---|---|---|---|---|---|---|---|---|
| 5 | stand_hills | 0.25 | 0.38 (0.43) | 0.106 / 0.047 | 898 | 5 459 / 7 354 | 53 | 8.9 |
| 5 | stand_mountains | 0.62 | 0.39 (0.45) | 0.130 / 0.060 | 1 107 | 5 857 / 7 927 | 56 | 12.6 |
| 5 | alt_100m | 0.58 | 0.39 (0.45) | 0.130 / 0.060 | 928 | 5 535 / 7 452 | 56 | 9.3 |
| 5 | alt_1km | 0.54 | 0.34 (0.42) | 0.103 / 0.053 | 756 | 3 489 / 4 891 | 55 | 7.1 |
| 5 | alt_10km | 0.25 | 0.26 (0.29) | 0.070 / 0.032 | 568 | 1 337 / 2 074 | 35 | 3.8 |
| 5 | alt_500km | 0.25 | 0.24 (0.35) | 0.056 / 0.024 | 398 | 273 / 384 | 0 | 0.4 |
| 10 | stand_hills | 1.00 | 0.73 (0.84) | 0.313 / 0.145 | 2 466 | 17 155 / 22 638 | 190 | 27.2 |
| 10 | stand_mountains | 1.08 | 1.10 (1.43) | 0.465 / 0.193 | 3 103 | 18 682 / 24 789 | 197 | 37.6 |
| 10 | alt_100m | 1.22 | 1.46 (2.26) | 0.499 / 0.215 | 2 486 | 17 903 / 23 575 | 195 | 28.8 |
| 10 | alt_1km | 0.77 | 1.12 (1.59) | 0.438 / 0.184 | 2 148 | 14 603 / 19 173 | 180 | 24.7 |
| 10 | alt_10km | 0.40 | 0.74 (0.95) | 0.290 / 0.115 | 1 634 | 5 319 / 7 378 | 138 | 11.7 |
| 10 | alt_500km | 0.42 | 0.48 (0.57) | 0.232 / 0.050 | 398 | 273 / 384 | 0 | 0.4 |
| 20 | stand_hills | 2.51 | 3.58 (3.78) | 2.03 / 0.74 | 6 738 | 56 457 / 75 547 | 980 | 85.8 |
| 20 | stand_mountains | 3.42 | 5.22 (5.88) | 3.07 / 1.11 | 8 602 | 62 461 / 83 527 | 1 040 | 116.6 |
| 20 | alt_100m | 3.17 | 5.30 (5.56) | 3.05 / 1.18 | 6 910 | 60 604 / 80 802 | 1 031 | 93.0 |
| 20 | alt_1km | 2.91 | 5.03 (5.38) | 3.02 / 1.10 | 6 816 | 60 699 / 80 285 | 880 | 93.7 |
| 20 | alt_10km | 2.26 | 4.19 (4.38) | 2.47 / 0.93 | 5 716 | 23 956 / 31 322 | 380 | 45.3 |
| 20 | alt_500km | 0.55 | 2.80 (2.91) | 1.79 / 0.58 | 398 | 273 / 384 | 0 | 0.4 |

Factor 10 is the one Andrew gets to choose: the main thread is 0.7–1.5 ms
average at every pose (p95 0.8–2.3 ms) against the 3.5 ms the naive set cost,
and the streaming update itself is 0.5–0.7 ms of that (the rest is encoding
2 000–3 000 draws). The tree step is linear in nodes at about 37 ns per
node (three passes over a 130-byte node); at factor 20 that is 2–3 ms for
80 000 nodes, and the GPU is at 2.5–3.4 ms with 4.5–5.6 M triangles: factor
20 is a GPU and memory question (127 MB of quads), not a main-thread one.
The 1 % drops (alt_500km at factor 20: 2.8 ms CPU for 273 chunks) are the
dead nodes of the collapsed tree still being scanned: the slab keeps its
peak length; compacting it is a small follow-up.

The fixed 60 s flight (50 m AGL at 50 m/s over the hills, climb to 5 km,
dive back; `M1b` idle number for the old streamer: 633 fps avg, 307 fps 1 %
low, worst 4.8 ms; the M1b-era run on a loaded machine: 747 / 188 / 24):

| world | factor | frames | avg fps | 1 % low fps | worst ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|---|---|
| 1/70 | 5 | 84 621 | **1 422** | **608** | 5.87 | 0.72 / 1.18 | 0.36 / 0.65 | 0.23 / 0.27 |
| 1/70 | 10 | 32 221 | 541 | 239 | 11.35 | 1.82 / 3.28 | 1.11 / 1.93 | 0.59 / 1.06 |
| 1/70 | 20 | 6 348 | 106 | 68 | 18.93 | 9.40 / 13.41 | 5.79 / 8.23 | 2.97 / 5.19 |
| **1/8**, H 4096, sea 1536 | 5 | 59 622 | **1 003** | **396** | 6.42 | 1.04 / 1.68 | 0.53 / 1.04 | 0.35 / 0.53 |

The 1/8 flight is the one the coordinator asked for: Opus measured it at
14 fps with the naive streamer, where every newly visited footprint paid its
height range on the main thread; with ranges in workers and the tree it is
1 003 fps average and 396 fps for the worst 1 %, worst frame 6.4 ms, with
10 500 resident chunks at the standing pose (the taller relief spans more
chunk layers). Worker costs with landforms v2: LOD-0 generate median 3.0 ms
(p95 3.7), mesh 0.2 ms, 16 threads.

## Not verified

- Timing on an idle machine. Everything above ran with other missions
  compiling and benching; the averages are stable across runs, the maxima are
  not. Austin's re-measure decides the ≤ 2 ms question at factor 10.
- The shader change for #11 is verified numerically and by the horizon crop
  at factor 0.2; it is not in the branch (render/ is Astra's). The `r0` the
  mesher writes is harmless until the shader reads it.
- GPU culling: not done; the measurements did not ask for it (GPU time at
  factor 20 is in the table).
- Mouse-driven sneaking: the harness test holds Ctrl; nobody has backed off a
  real edge with the keyboard.
- Edits across a cube *corner* (the one diagonal padding column where three
  faces meet) are left natural in coarse padding; documented in `overlay_coarse`.
- `Enclosed` chunks that get edited are rendered through the tree path
  tested above (the band test admits any chunk with edits, an invalidation
  for a chunk that is not a node creates it under its split ancestor); it
  has not been dug by hand in the app. The LOD-0 chunk appears one level at
  a time as the tree refines, so a shaft dug straight down 100 m shows its
  walls coarsely for the first frames.

## What I would change in the plan

1. Footprint ranges are the main-thread cost now, not traversal; keeping them
   in workers (done) makes the tree's own cost ~0.2 ms at 8 k nodes. If
   landforms make them pricier still, the range jobs could compute all four
   children at once.
2. The horizon ring: a child that comes over the horizon unloaded makes its
   parent draw again (one LOD coarser for the frames until it loads). It is
   correct and invisible in the flights; if it ever shows, pre-dispatch
   hidden children within one LOD of the horizon.
3. `settle` for a shot waits for every range job and every split; a shot
   over new territory takes 0.1–1.4 s (it was 0.0–1.0 s).

## Feedback

- **New tooling used:** the `travel` command is what made the coverage tree
  reviewable: a strict per-frame invariant check during real motion found
  the one real bug (a revert left split nodes below the cut) in the unit test
  first and then confirmed zero violations in flight. It also exposed the
  range cost, which the static poses never would have. Keep `travel` in every
  streaming change's script.
- `Coverage` runs without a GPU or a world (closures for ranges and
  classification, `Mesh::Gpu` with fake slots), so the invariant test runs in
  about a second and can be extended with new camera paths cheaply.
- The sandbox refuses any shell line that `cd`s before a git command, even
  `cd .. && git ...` from inside `game/`; run git from the worktree root with
  paths. Cost me three retries.
- `gen` is a reserved identifier in edition 2024; the compiler tells you,
  but a sweep of the file is quicker than fixing each site.
