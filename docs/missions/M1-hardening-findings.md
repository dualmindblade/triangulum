# M1-hardening: the rest of Astra's second review (Fable, 2026-10-25)

Branch `worktree-agent-ae99c5b30c1596805`, crate `game/`, from main at
491dc57 (M2a merged). Findings #6, #7, #8, #9 and #10 of
`docs/reviews/2026-10-25-engine-review2-astra.md`. Evidence in
`docs/missions/M1-hardening-evidence/` (downscaled frames and logs) and,
full size, under
`C:\code\triangulum\.claude\worktrees\agent-ae99c5b30c1596805\game\out\play\`
(gitignored, this machine only). Nothing in `world/` or `worldgen/` was
touched; `render/shader.wgsl` was.

## Each finding: what was wrong, the fix, the test that failed before it

Every test below was run twice on the same test code: with the five fixes
reverted (`FIXES-OFF` in the evidence logs: `fixes-off-tests.txt`, a
scripted revert of exactly the fix hunks, tests kept) and with them in
(`tests-release.txt`). "Fails before" is that first run, not a thought
experiment.

### #6 Breaking across a face edge changed the wrong block (`player.rs`)

`pick` looked blocks up through `Voxels::block`, which resolves a cell
past the face edge onto the neighbouring face, but built `Hit.block` by
clamping the frame cell onto the frame's own face. The hit said face 2's
material; `break_block` edited face 0's boundary column.

Fix: `Hit.block` is `Planet::wrap_cell` of the frame cell, and a solid
below layer 0 (the virtual bedrock floor `block_lattice` returns) is no
hit instead of being clamped to layer 0.

Test `player::tests::picking_across_a_face_edge_names_the_neighbours_block`
(a `Canon` world that resolves cells the way `World` does): a ray from face
0 two columns past its +i side hits the face-2 block and names it
`BlockPos { face: 2, .. }`; looking down from layer 0 onto the virtual
floor is `None`; a real block at layer 0 is still picked. **Fails before**
(`hit.block` was `{ face: 0, i: N-1, .. }`), passes after.

### #7 Missing tunnel walls after a reload (`stream.rs`)

Admission was `classify_in(key) == Mixed || edits.any_under(key)`. Digging
the border block of chunk A exposes a face of the block beside it in chunk
B, which only B's mesh draws; B is naturally Enclosed and has no edits of
its own, so after a reload (the tree is rebuilt from the roots through
that test), or after the camera leaves and comes back (a merge, then a
split through that test), B is never a node and the wall is a hole. Live
it works because `materialise` creates the edited key under its nearest
existing split ancestor without asking the band test (the script's live
`cover_rel` is `drawn:0` with the fixes off too); it is the reload and
the merge-then-split that lose it.

Fix: `stream::chunk_exists`: Mixed, or `edits_touch_halo`: an edited
LOD-0 chunk inside the chunk, or a block in its one-voxel halo whose
stored id differs from the generator's (exact in blocks on the chunk's
own face; conservative across a face edge, where any edited chunk under
the neighbouring face's chunks along the padding strip, at the right
layers, admits it). Because an edit adjacent to B is in the padding of
every ancestor of B that does not contain it, the whole chain refines
down to B.

Test `stream::tests::a_chunk_whose_padding_holds_an_edit_exists` (real
bake): under the hills, the first naturally Enclosed chunk below the
surface chunk is not a node; digging the block on the chunk above's
bottom layer makes it, and every ancestor of it, exist; two chunks down
stays out; the sideways neighbour is admitted only when the dig is in its
first column; across the face-0/face-2 edge, a block dug 40 layers under
the ground in face 0's last column admits the face-2 chunk whose padding
holds it. **Fails before** (`chunk_exists(below)` false after the dig),
passes after.

Harness: `cover[F,I,J,K]` and `cover_rel[DI,DJ,DK]` report what the
streamer's cut shows at a block after a shot (`drawn:L`, `empty:L`,
`gap:L`, `hidden:L`, `none`; `Streamer::cover_at`), and
`scripts/tunnel-wall-reload.play` digs a 26-block shaft at (face 2,
107672, 57752) down to layer 512, the bottom of LOD-0 chunk cz 16, over
the Enclosed chunk cz 15, saves, reloads, and asserts `cover_rel[0,0,-1]
== drawn:0` live, after the reload and from the top of the shaft. The script
needs a shaft that deep because `footprint_height_range_m` carries a
relief margin: no column near the hills has an Enclosed chunk closer than
25 layers under its ground block (a probe over 1,200 x 1,100 columns).
**With the fix: 18 asserts, 0 failed** (`tunnel-wall-reload.log`,
`tunnel_03_bottom_reloaded.jpg`, `tunnel_04_down_the_shaft.jpg`: the shale
floor is there). **Fixes off: 2 failed** (`fixes-off-tunnel.log`): live
`drawn:0`, after the reload `cover_rel[0,0,-1]` is `none` at the bottom
and from the top (`tunnel_03_bottom_reloaded_FIXES_OFF.jpg`,
`tunnel_04_down_the_shaft_FIXES_OFF.jpg`: no floor, the view falls
through into the dark).

### #8 A stale mesh stranded after an allocation failure (`stream.rs`)

`Streamer::install` tried the arena before checking the result's
revision. A result that had gone stale in flight, found no room and was
parked, was then found not current on retry, unparked (`job = None`) and
dropped; the node kept its old resident mesh, and nothing re-requested a
resident node, so the stale mesh stayed and `settled()` said yes. A
current result dropped at the awaiting-queue limit stranded the same way.

Fix: `Node::mesh_rev`, the revision of the resident mesh, set by
`install`; every step, a resident node with `mesh_rev != rev`, no job and
in view asks for a job and counts as unsettled. `Coverage::validate`
checks a result (dead, stale, or current) before anything is allocated:
`Streamer::install` re-dispatches a stale one and drops a dead one before
touching the arena, so neither can be parked; the retry path is the same
`install`. A result dropped at the queue limit is counted and the node
asks again through the step rule.

Test `stream::tests::a_stale_resident_mesh_is_requested_again_when_its_replacement_is_lost`
walks the streamer's exact old sequence on the tree (invalidate, invalidate
again, park the first result, find it not current, unpark): the tree must
be unsettled and ask for the node at its current revision; the new result
installs and the old mesh is freed; `validate` answers a stale result with
the current job without touching the mesh and drops a wrong epoch; a
parked-then-dropped current result is asked for again; no GPU mesh leaks
or double-frees. **Fails before** (no want for the node, unsettled 0),
passes after. The random-camera invariants test still passes with the
new rule.

### #9 Water seams at real triangle edges (`mesh.rs`, `render/shader.wgsl`)

Right as stated: the exact-radius patch put every vertex on the sphere,
but a merged eight-voxel sea quad at LOD 9 is a 4 km chord that sags 23 m
mid-edge below the LOD-8 neighbour's vertex there. Measuring the emitted
edges (not a hypothetical midpoint) found a second, larger term: the
curvature grid is bilinear over 4x4 cells of 8 voxels, the equal-area
lattice lines curve, and a mid-cell vertex of the LOD-9 chunk sat up to
**30 m** beside the LOD-8 chunk's corner on the same lattice point, a
lens of overlap or gap along every coarse boundary.

Fix, two parts:
- `mesh::SEA_EDGE_MAX_M = 256`: the tangent edges of every water quad and
  of every quad on the sea-level plane (at LOD 9 the sea is the opaque
  `deep_ocean` voxel, and a beach at sea level is the same plane) are
  capped at 256 m: one voxel at LOD 9 and 8, 2 at LOD 7, 4 at LOD 6, the
  grid cell at LOD 5, the whole chunk from LOD 2 down. From LOD 5 up the
  cap is a voxel multiple at both LODs of a boundary, so edge vertices
  coincide and there is no T-junction; the remaining chord is one LOD-9
  voxel (0.5-0.6 m sag).
- The shader interpolates the grid with the quartic through the chunk's
  five corners along each axis at voxels of 64 m and up (`lagrange5`,
  25 taps instead of 4, only at LOD 6 and coarser). Two chunks of one
  LOD still evaluate a shared edge from the same five corners, so nothing
  opens between them; the residual against the lattice's own curve over
  16 km is up to 2 m along the edge at LOD 9 (the warp's higher
  polynomial terms), 0.05 px at the 41 km the default LOD factor draws
  LOD 9 from.

Test `mesh::tests::emitted_sea_edges_meet_across_an_lod_boundary` (real
bake): along three LOD-9/LOD-8 boundaries on face 0 that have sea on both
sides, every emitted sea-quad edge of the coarse chunk's +x side is taken
as the chord between its two vertices (placed as the shader places them,
cap and quartic modelled in `vertex_pos`), and every fine-chunk sea vertex
on that line is measured against it. **Fails before: worst separation
46.8 m, 35.9 m of it radial** (cap off, bilinear grid; 21 fine vertices
against the eight-voxel edges, the chord of a stretched LOD-9 cell being
longer than 4 km where the lattice is wide); after: 147 vertices, 2.02 m
total, 0.58 m radial. The older midpoint test still passes.

Frames (`seam_*.jpg`, pixel diffs in `seam-pixel-diff.txt`):
- `--lod-factor 0.2`, 1.5 km over the deepest ocean at local noon
  (`water-lod-seam.play`): the thin light step lines along the LOD-8/9
  band near the horizon are gone (`seam_f02_band_zoom.jpg`, 2x). The
  dark stair-stepped lines in the near field are **not this finding**:
  they are unchanged by the fix (pixel-identical), sit where the sag is
  centimetres, and are the chunk-edge skirt walls seen through the water
  (see Not verified / for the ledger).
- Default factor from 30 km (`water-lod-seam-high.play`, new): the LOD-8/9
  boundary is 40 km out, where 23 m was 0.6 px; before and after differ
  in 0.12 % of pixels, spread along the ocean. Nothing visible changed,
  which is what a sub-pixel seam fixed sub-pixel looks like.
- Orbit (`ocean-orbit.play`, 150 km): 0.03 % of pixels differ; the
  opaque deep ocean of #12 still backs the sea (`ocean_orbit150_*.jpg`).

Cost: the coarse-LOD vertex path from 30 km, where every drawn chunk is
LOD 7 or coarser (`bench 120` in `water-lod-seam-high.play`, 218 draws
both ways): fixes off, wall avg 0.50 ms, GPU avg 0.23 ms; with the cap
and the quartic, wall avg 0.90 ms (p95 0.89, worst 2.04), GPU avg
0.61 ms. About +0.4 ms of GPU time in the heaviest coarse view, from the
sea's one-voxel quads (up to 1,024 top faces per LOD-9 ocean chunk
instead of 16) and the 25-tap grid read; PROVISIONAL, other missions
share this machine (`seam-after-f5.log`, `fixes-off-seam-high.log`). The
ground-level flight bench is unchanged in kind (`edge-corner-flight.log`:
0.99 ms wall, 0.49 ms GPU, 683 draws, against M2a's 0.37-0.64 ms main
thread under load).

### #10 Collision at face transitions (`player.rs`)

The sweep ran in the old face's lattice (cells past the edge through
`wrap_cell`) but the position stepped in the tangent plane. The two
lattices meet at the edge with a kink, so the box, re-expressed in the
neighbour's lattice, could overlap a cell the sweep never checked:
Astra's case, feet at face 0 (N-0.02, N-0.70, 600) walking +i along a
wall in row N-2, ends the crossing step with the box inside the wall and
is stuck there.

Fix: a move that leaves the face is split at the edge (`edge_fraction`);
the rest is swept in the neighbour's own lattice (`Frame::from_lattice`
at the wrapped edge point, the remaining world displacement re-expressed
there, `sweep_horizontal` with the same sneak rule); then the whole box
is checked in the frame it ends in (`free_box`) and, if it leans into a
cell, nudged out sideways by the smallest amount that frees it (at most
half a cell), or, failing that, the horizontal move is refused for the
step. A box that straddles the edge without its centre crossing is still
swept in its own face's lattice (README, known gaps).

Tests `player::tests::walking_across_a_face_edge_along_a_wall_keeps_the_box_out_of_it`
(Astra's numbers on a `Canon` world with the wall in row N-2 continued
onto face 2: after every step the box overlaps no solid cell in the
lattice of its own position, and the walk gets through at walking speed)
and `a_wall_just_past_the_edge_stops_the_walk_without_overlap` (a wall in
face 2's second column: stopped on face 2, never overlapped). The first
**fails before**: on the crossing step the box, in face 2's lattice,
overlaps the wall cell by 0.0054 lattice units (Astra's 0.0255 is the
same mechanism at their exact geometry; mine puts the wall one row
nearer), and passes after. The second passes with the fixes off as well:
it is a guard that the split sweep does not break the plain case, not a
repro. The existing edge and corner walks are unchanged.

### Optional: the incremental active-region recompute

Not done. 0.86 ms per chunk crossing stands; at walking speed that is once
every seven seconds. It is a contained change in `sim/mod.rs`
(`refocus`: keep the set and add/remove the ring the player's chunk
change exposes) and did not fit the verification budget of this mission.

## Evidence

- `tests-release.txt`: `cargo test --release`, **98 passed, 0 failed,
  2 ignored** (the stress test and the 5,041-block set), on the committed
  tree after the fixes-off pass was reverted.
- `fixes-off-tests.txt`: the same tests with the five fixes reverted:
  **93 passed, 5 failed**, the five being
  `picking_across_a_face_edge_names_the_neighbours_block`,
  `walking_across_a_face_edge_along_a_wall_keeps_the_box_out_of_it`,
  `a_stale_resident_mesh_is_requested_again_when_its_replacement_is_lost`,
  `a_chunk_whose_padding_holds_an_edit_exists` and
  `emitted_sea_edges_meet_across_an_lod_boundary`; every pre-existing
  test still passes there, so nothing else depends on the fixes.
- `m1-done.log` 94/94, `m2a-sand.log` 95/95, `edge-corner-flight.log`
  17/17 (cover violations 0 on all three legs; main-thread cpu avg
  0.38 / 0.45 / 0.65 ms; my own test build ran beside the flight, so the
  timings are provisional, the asserts are not).
- `tunnel-wall-reload.log` 18/18; `fixes-off-tunnel.log` 16/18, the two
  `cover_rel[0,0,-1] == drawn:0` asserts after the reload failing with
  `none`.
- `seam-after-f02.log`, `seam-after-f5.log`, `ocean-after.log`: the frame
  runs; `seam-pixel-diff.txt`: before/after pixel counts.

## Not verified

- The windowed app: the harness and the tests drive the same `Session`,
  `Streamer` and shader; nobody has walked across a face edge along a wall
  with the mouse, or watched the sea from orbit on a screen. The frames
  are offscreen renders of the same pipeline.
- The quartic path's GPU cost on a quiet machine: the bench ran with
  nothing else of mine running, but other missions share this machine
  (the numbers are labelled).
- Allocation failure in the real renderer: #8's test drives `Coverage`
  through the streamer's exact call sequence, not a full `Renderer` with
  a full arena. The streamer's own change (validate before allocate, retry
  through `install`) is covered by the scripts' normal paths only.
- The near-field stair-stepped lines over water at `--lod-factor 0.2`
  (`seam_f02_before_down.jpg`, bottom half): present before and after,
  so not part of this finding; they look like the chunk-edge skirt walls
  (`worldgen`: border columns hold plain water so edge voxels grow side
  walls) drawn through the translucent water surface of the neighbour.
  Not diagnosed further; for the ledger.

## Feedback

- `cover_rel[DI,DJ,DK]` was the missing assert for "is this drawn": the
  stream's resident set alone cannot say whether a block is covered by a
  drawn node with a mesh, and walking the ancestors to the cut does.
  Worth keeping for every future hole report.
- A test that measures the emitted edges instead of a hypothetical
  midpoint found the bilinear grid's 30 m, which the midpoint test could
  never see. "Test the thing the GPU draws" was the right instruction.
- The `Canon` test world in `player.rs` (cells resolved through
  `wrap_cell`, like `World`) is what the edge tests needed; `Flat`
  ignored the face and could not express "a wall on face 2".
- The shaft probe (an ignored test, deleted) was the quickest way to find
  a scriptable site; the relief margin in `footprint_height_range_m`
  makes "Enclosed" much deeper than "below the ground", which is worth
  knowing before writing a dig script.
- The worktree sandbox refuses compound shell lines that mention `git`
  anywhere, including `git -C <this worktree>`; plain separate commands
  work. Harmless once known.
