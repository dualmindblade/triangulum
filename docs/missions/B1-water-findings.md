# B1 water surfaces: findings

**Status: the fixes are written and committed, and they compile. None of them
has run.** The Visual Studio C++ tools were removed from this machine during
the mission, so nothing can link. Everything under "Measured" came from the
unfixed build. Everything under "Owed" is unproven until the linker is back.

Evidence is in `docs/missions/B1-water-evidence/`.

## What was wrong

### #1, water under a solid

Andrew's "slab" in recording 040850 is a one-block-wide stone wall in layer 513,
sitting over a river whose top cells in layer 512 hold levels 105 to 123. So
there is half a metre of air between the water and the stone. This is not the
coplanar case the bug table expected. The top face was culled because the cell
above was solid, so a strip of surface half a metre below the stone was missing
and you looked through it at the bed.

Fix: a top face hides only under water, or under a solid whose underside it lies
in. A full cell under a solid stays hidden, because its top would z-fight with
the solid's underside and tint a view from below twice.

`scripts/b1-water-under.play` builds this on a generated river. The pre-fix
frames show the hole from all three views (`1_above_before.png`,
`1_side_before.png`, `1_underwater_before.png`).

### #10, the dip at the shore

**The bug table's reading was half right.** Air does count as height 0, but at a
resting shore the air does not exist. The mesher reads padded cells, and the
generator lowers a chunk's border columns by its 2-voxel skirt so the terrain
grows side faces that hide LOD seams. So the next chunk's ground reads as air.

The `dips` audit (a temporary harness command, not committed) lists every
top-face corner below its own cell's level, and what pulled it down:

| Site | Dipped cells | Skirt ghost | Real air |
|---|---|---|---|
| `find lake`, no edits | 22 | 22 | 0 |
| Andrew's river | 71 | 54 | 17 |
| m2b-river site | 72 | 64 | 8 |

- At the lake, all 22 dipped cells are on chunk borders.
- Every real-air cell in all three river samples is over air, none over water
  or ground: the lip of a waterfall. That is M2b's deliberate slope, so real air
  still counts.
- The sea shores sampled have no dips, because a chunk with no partial water
  draws flat.

`10_dip_before.png` is the bug: a V pulled down at a chunk corner, 47 levels
(18 cm), with tan slivers along the shore.

**Recording 040850 does not show this bug.** Its "dark wedges" have the same
shapes with the water hidden (`10_wedges_040850_with_water.png`,
`10_wedges_040850_water_hidden.png`): they are riverbed steps seen through the
water. The `find lake` site above is the real reproduction.

**Why it went away on update.** The simulation does not fill that air: those
cells are sand, asserted before and after. `overlay_lod0` copies an edited
neighbour's true cells over the skirted padding. So a dip clears when the chunk
holding the ghost cell is edited and the dip's own chunk remeshes. Four runs at
one corner where four chunks meet (i 115488, j 64096):

| Action | Corner | Audit | Frame |
|---|---|---|---|
| Nothing | `a0 a0 a0 w204` | dip 47 | V visible |
| One bucket, stays inside one chunk | unchanged | dip 47 | not looked at |
| Three stones on dry sand, simulation off | ghosts gone | 0 | **V still there**: nothing remeshed the chunk |
| Water poured across the chunk borders | `a0 s0 s0 w214` | dip 19 | nearly flat (`10_dip_after_border_pour_no_fix.png`) |

The 19 that remains is the ghost in the fourth chunk, which is dry and which
water never reaches. So Andrew's "goes away" is real but partial, and depends on
where he poured. The stones row shows the audit alone was not enough: it
recomputed a mesh the streamer never rebuilt.

Fix: the terrain still needs the skirt, so the padded ids stay.
`World::padded_levels` now also lists the border cells that are really solid,
and only the water surface reads them. Face emission is unchanged. The lookup
runs only for chunks with partial water, and only in the layers beside a surface.

### Review 3 #8, gaps at joins

- **Chunk joins.** A corner with nothing partial at it is now full, air or not,
  which is what the flat shortcut draws. The skirt was a third source of
  disagreement here, beyond the two Astra listed: each chunk saw the other's
  border columns lowered.
- **Cube corners.** The padded diagonal does not exist there. It is now left out,
  so the three real cells count once each.

### Hairlines: cause found, not fixed

**It is not the sea, and not a crack.** The surface in recording 041049 is a lake
at +12.03 m. `block_at` rounds it to the voxel grid separately at each LOD:

| LOD | 0 | 1 | 2 | 3 | 4 | 5 and up |
|---|---|---|---|---|---|---|
| Water top (blocks above sea) | 12.03 | 12 | 12 | 16 | 16 | none (#2) |

So there is a 4 m step at the L2/L3 boundary with no wall, and from inside the L2
ring you look under the L3 sheet at the bed.

| Test | Hit pixels (4K) |
|---|---|
| As shipped | 4605 |
| Sag threshold moved | 4606: not the cause |
| Water depth test off | 4605: no water is drawn there at all |
| **L2 water raised one voxel, level with L3** | **0** |

Doubling the resolution gave 3.87× the pixels, which is a fixed-width gap in the
world, not rasterisation. At those pixels the water frame and the no-water frame
are identical. Vertices on the shared edge agree to 2 mm.

The rounding is `w_r` in `block_at`, in B2's file, so it is left alone.
**Proposed fix, after B2 lands:** give coarse LODs a fractional water level, so
each draws the true surface inside its top voxel, as LOD 0 does. The water quad
format already carries the height. It needs `Generated::levels` at coarse LODs,
and the `lod == 0` filter in `mesh_chunk_levels` lifted. Any lake off the grid
has this, by up to half the coarser voxel. If B2 only keeps lake water at coarse
LODs (#2), the same step will appear at more boundaries.

### The three "only if they share a cause"

- **Stair-step: reproduced, no shared cause, not fixed.** Drawn identically with
  water hidden (`stairstep_with_water.png`, `stairstep_water_hidden.png`). It is
  a seabed terrace seen through the water, at L5 and coarser.
- **Hill crack: not reproduced, not attempted.** M3b found it in opaque terrain
  quads. None of this work touches them.
- **Dark river triangle: not reproduced.** No recording or pose exists for it.
  The triangles in `10_dip_before.png` could be the same thing, but I can't show
  that.

## Owed once the linker is back

Nothing below has been done.

- [ ] `cargo test --release`, including the six new tests in `mesh.rs`
- [ ] Each new test failing with its fix off
- [ ] After frames, same poses: `b1-water-under.play`, the lake corner, 040850
- [ ] Scripts: `m1-done`, `m2b-water`, `m2b-boundary`, `m2b-river`, `m3a-tunnel`,
      `water-lod-seam`, `water-lod-seam-high`
- [ ] Quad counts on an ocean and a lake view, before and after (provisional)
- [ ] The cost of the border lookup in `padded_levels`, and of a natural-cache miss
- [ ] Replays still reproduce on the fixed build

## Risks in the fixes, unproven either way

- `chunks_agree_on_every_shared_water_corner` may fail on the #10 commit alone
  and pass only with #8, if those sites hold a full cell beside real air. Taking
  the two commits separately could expose that.
- The border lookup can generate a neighbouring chunk on a cache miss, on a worker
  thread. I expect that to be rare, but have not measured it.

## Not verified at all

Anything in the real window. Every frame here is harness-rendered, as the round
requires.

## Build notes for the lead

The binary behind every measurement here was linked with Microsoft's NuGet import
libraries, because the SDK was already gone. It loads the same system DLLs, and
the unfixed build matched the recording: replay 040850 reproduced exactly.

## For Andrew

Nothing to decide yet. One look change to judge once this runs: full water beside
air no longer slopes at its edge. A chunk with no partial water already drew it
this way. Simulated water normally spreads out of that state within a tick, but
frozen water can hold it, as the placed column at i 109775 in his 040850 save
does.
