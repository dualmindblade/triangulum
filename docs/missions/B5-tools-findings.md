# B5-tools findings

Bug round rows #6 and #11 (`docs/missions/2026-10-30-bug-round.md`).

## #6, the mesh view

### What it is

Austin's correction: Andrew did not mean a plain wireframe. He wants the mesh
**coloured by region**, so the boundaries between regions show, as a debug view
you cycle through. Lines stay, because they combine with the colours and B1
needs them. It is not a smooth distant mesh and it does not change the normal
view.

Two settings that combine. **M** steps the colours, **Shift+M** the lines, and
both are in the look panel under *Mesh view*. Each names itself at the top of
the screen with a legend of its colours.

| Colour by | The region | Where the data comes from |
|---|---|---|
| LOD level | each level of detail | the draw's voxel size |
| Chunk | every chunk: a 3D checkerboard of light and dark, so chunks sharing a face always differ, in a hue of its own | a word kept per chunk at upload |
| Cube face | the six faces | the same word |
| Quad | every greedy quad | the quad's number |
| Block | each kind of block, water included | the quad's block id |
| Biome | the Köppen class | the bake's `koppen` layer |
| Tectonic plate | the plates (11 on this world) | the bake's `plate` layer |
| Crust | oceanic or continental | the bake's `crust_type` layer |

"Regions" could mean more than Austin's four examples, and the bake holds three
regions the engine had never read, so Quad, Plate and Crust are modes of their
own instead of a guess. Biome is the bake's Köppen class, and Block shows the
surface biome as the generator actually placed it; the two differ, and seeing
that is useful (see "Worth a look").

Lines: *none*, *edges* (over any picture), *edges only* (dark terrain, black
sky, so a crack in the mesh is a black hole in the slate). Over a colour the
lines are dark; with edges only they take the region's colour; with no colour
chosen they take the LOD's. A heavy line is a chunk's border and the thin
diagonal shows how a quad is cut into two triangles. Water has no fill in edges
only, so its quads lie over the bed under them.

### How it's built

- Edges are found in the fragment shader from coordinates carried across each
  quad, not with `PolygonMode::Line`. That needs a device feature the game
  doesn't otherwise require, gives one-pixel lines, and can't tell an edge from
  a diagonal. This works the same on Vulkan and DX12.
- Every position comes from `shader.wgsl`'s own `vertex()`, so sloped water
  corners are outlined where they really are. `mesh_view.wgsl` reads the water
  quad's two-slot layout itself, in one place (`mesh_vertex`); if B1 changes that
  layout, that line changes with it.
- The bake's three byte layers are read by `render/regions.rs`, off the main
  thread, when a tint first asks (1 MB a layer; 24 MB of texture, only then).
  `worldgen/` wasn't touched. The shader looks them up per pixel by the bake's
  own rule, so a region's edge is the bake's real edge, cell for cell.
- The legend and the shader read one colour table (`meshview::palette`).

### The off case is unchanged

The normal pipelines and their shader module are untouched: the view's code is
in its own module, built the first time the view is switched on, and `render`
gains one comparison. `DrawRecord.pad` now carries the chunk's word, set once at
upload and read by no normal pipeline.

`--bench --skip-flight --frames 200`, the frozen `945d544` build against this
one, all seven poses:

| Pose | PNG | GPU ms before / after | CPU ms before / after |
|---|---|---|---|
| stand_hills | byte-identical | 1.76 / 1.35 | 1.38 / 1.30 |
| stand_mountains | byte-identical | 1.71 / 0.80 | 2.11 / 1.18 |
| alt_100m | byte-identical | 1.39 / 0.76 | 1.37 / 0.95 |
| alt_1km | byte-identical | 1.51 / 1.27 | 1.32 / 0.87 |
| alt_10km | byte-identical | 1.60 / 1.08 | 1.05 / 0.85 |
| alt_100km | byte-identical | 0.97 / 0.65 | 0.55 / 0.63 |
| alt_500km | byte-identical | 0.64 / 0.87 | 0.64 / 0.75 |

Timings are **provisional**: other agents were running, so the differences are
noise, in both directions. The byte-identical frames are the real evidence.
`b5-meshview.play` also switches the view off after using it and compares: the
frame is the same, pixel for pixel (`compare_same == true`).

### Evidence

`B5-tools-evidence/`: `mesh-regions-1500m.jpg` (every region from one pose),
`mesh-lines.jpg`, `mesh-cube-corner.jpg` (three faces meeting at lat 35.26, lon
135), `mesh-sloped-water.jpg`, `mesh-coast-orbit.jpg`. All from
`scripts/b5-meshview.play`, which also asserts that each tint and line setting
changes the picture.

`cargo test --release`: 172 passed, 0 failed, 14 ignored, on the #6 commit.
`m1-done`: 94 asserts, 0 failed. `m3a-look`: ran clean. `m2c-replay`: 29 asserts,
0 failed.

### Worth a look

- **Biome against Block.** At the hills the bake's Köppen class runs to EF (ice
  cap) on the high plateau where the generator's own rule gives grass and snow
  on a different line: the two modes side by side show it. That is the game's
  climate rule against planetgen's, not a fault in the view. Andrew's to judge
  whether they should agree.
