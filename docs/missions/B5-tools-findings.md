# B5-tools findings

Bug round rows #6 and #11 (`docs/missions/2026-10-30-bug-round.md`).

**State of this file:** the linker disappeared from this machine part way through
#11 (the Visual Studio C++ workload was removed along with the SDK). Sections
marked *pending the linker* were not run on the final code. See "Verified, and
how".

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
| Quad | every greedy quad | the quad's number in its chunk |
| Block | each kind of block, water included | the quad's block id |
| Biome | the Köppen class | the bake's `koppen` layer |
| Tectonic plate | the plates (11 on this world) | the bake's `plate` layer |
| Crust | oceanic or continental | the bake's `crust_type` layer |

"Regions" could mean more than Austin's four examples, and the bake holds regions
the engine had never read, so Quad, Plate and Crust are modes of their own
instead of a guess. "Block or biome" became both: Block is what the generator
placed, Biome is the bake's Köppen class.

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
  a diagonal.
- Every position comes from `shader.wgsl`'s own `vertex()`, so sloped water
  corners are outlined where they really are. `mesh_view.wgsl` reads the water
  quad's two-slot layout itself, in one place (`mesh_vertex`). **If B1 changes
  that layout, that line changes with it.**
- The bake's byte layers are read by `render/regions.rs`, off the main thread,
  when a tint first asks (1 MB a layer; 24 MB of texture, only then). `worldgen/`
  wasn't touched. The shader looks them up per pixel by the bake's own rule, so a
  region's edge is the bake's real edge, cell for cell.
- The legend and the shader read one colour table (`meshview::palette`).

### The off case is unchanged

The normal pipelines and their shader module are untouched: the view's code is
in its own module, built the first time the view is switched on, and `render`
gains one comparison. `DrawRecord.pad` now carries the chunk's word, set once at
upload and read by no normal pipeline.

`--bench --skip-flight --frames 200`, the frozen `945d544` build against the #6
commit, at the seven poses:

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
noise, in both directions. The frames are the real evidence.

`b5-meshview.play` also switches the view off after using it and compares: the
same frame, pixel for pixel (`compare_same == true`).

### Evidence

`B5-tools-evidence/`: `mesh-regions-1500m.jpg` (every region from one pose),
`mesh-lines.jpg`, `mesh-cube-corner.jpg` (three faces meeting at lat 35.26, lon
135), `mesh-sloped-water.jpg`, `mesh-coast-orbit.jpg`, all from
`scripts/b5-meshview.play`, which asserts that each tint and line setting changes
the picture. Sloped water: a trench with a cell of water let go at one end, three
ticks on; each water quad shows in its own colour, with its side slivers.

On the #6 commit: `cargo test --release` 172 passed, 0 failed, 14 ignored.
`m1-done` 94 asserts, `m3a-look` clean, `m2c-replay` 29 asserts, 0 failed.

Both backends run the script clean (RTX 2060, Vulkan and DX12; 11 asserts each).
Frames match to within a few dozen pixels, except that Quad's colours differed
completely: they came from the quad's place in the shared buffer, which depends
on streaming order. Fixed (they now count from the chunk's first quad); *pending
the linker* to confirm.

## #11, the T window

### What's there

**T** opens one window; T or Esc shuts it.

- **The map** (`map.rs`): re-synthesised from the bake for the exact view, zoom
  1 to 180× anchored on the cursor, drag to pan. Base layers Biomes, Temp, Precip,
  Elevation with adaptive contours (1-2-5 spacing in game metres, every 4th
  stronger, the spacing written on the map). Relief, rivers, lakes, markers.
  Clouds, wind, the moon tab and the multiplayer row are left out, as asked.
- **The photo roll** (`photos.rs`): every `shot_*.png` is a marker and a row, with
  a preview over the map, a lightbox, editable Title and Notes, and delete to
  `trash/`, the photo and everything named after it, all or none.
- **Teleport**: to a photo, a map point, or typed `lat lon [alt]`, with optional
  restore of the time of day. You-are-here and a free target marker.
- **The places**, off P, as a row of buttons.

**Screenshots.** P is a settled, clean photo; Shift+P is this frame with the HUD.
Both go to `game/interchange/` (now in `.gitignore`). M3a's Screenshot button and
`--screenshot` write the same format, so there is one format.

**Repro.** `--repro SHOT.png` or the window's *Repro* button writes `SHOT.play`
with the exact pose and clock, the view, a settled clean frame, a comparison with
the original, and checks of the edit hash and generator version. It prints the
full command line and an `--at` command.

**For B6**: a sidecar is only ever read, changed and written back
(`photos::edit_sidecar`), so a new key survives every other edit, and a test
proves it. Delete takes every file named after the photo. The editor is
`Menu::ui_photo_editor`, the lightbox `Menu::ui_lightbox`.

### Decisions worth checking

- **P holds the game still** until the view settles, for at most 20 s: no steps
  pass, so neither the simulation nor a recording can tell, and the frame stays
  responsive (streaming keeps running and the screen says how many chunks are
  left). Input queues. Standing still that is about 10 frames; after a teleport,
  seconds.
- **Temp and Precip are yearly means.** §17.3 says current season, but this engine
  has no seasonal climate yet. The window says so.
- **Biomes on the map** is the block on the ground by `Generator::surface`'s
  climate thresholds (softened), in Andrew's block colours. It isn't Köppen,
  and it will drift if B2 changes those thresholds. Lakes come from the
  generator's own columns; rivers are vector lines from `hydro.bin`.
- **Zoomed in the map goes soft.** The bake's cells are about 440 m on this planet,
  so by 36× it's a smooth wash with sharp rivers. A build from real columns would
  be sharper, and slower; I left it.
- **Teleport with no altitude** stands on the ground, except over water deeper
  than 1.5 m, where it hovers 30 m above the surface: `place_on_ground` would
  otherwise put you on the sea bed. Altitude clamps to 2.5 m above ground and
  80,000 km, as §15.7 says.
- **Typed altitude is metres**, like `--at` and `at`; the viewer's was km.
- **While the T window is open the look panel is hidden**, not closed, and F1 is
  ignored. **Esc now closes the look panel** when no popup is open: a layer I added
  for §17.1. It costs nothing, but it is a change to M3a's behaviour.
- **A photo from another world** (different seed, dials or bake) goes to the same
  latitude, longitude and height, and the button says so.
- `build.rs` is new: it stamps the commit into the binary for the sidecar's
  `build`.

### Determinism

Teleports and places go through `Controls::queue` and `Session::apply` as `pose`,
`sky` and `preset`, at a step boundary. The replay format did not change. New tests
(`replay_tests.rs`, *pending the linker*): a teleport by coordinates, a place, a
map point and a photo with its clock, during a recording, replay exactly with no
notes; old replays parse and `preset` plays; P queues nothing; a session with
photos taken along the way gives the same hash and the same file as one without.

Andrew's five recordings, played headless on the last linked build (the T window
code in, P no longer a preset): all five reproduced exactly, 49 of 49 checks and
all five end hashes.

### Repros, measured

Fresh process, from the sidecar alone, on the last linked build:

| Photo | Pixels different from the original | |
|---|---|---|
| hills, 1.5 km up | 0 of 2,073,600 | byte-exact |
| coast, 300 m up | 3 of 2,073,600, by at most 10 | at the far horizon |

Three repro runs of the coast are byte-identical to each other, so this isn't
noise: the original arrived there by teleport from the hills, and a settled frame
keeps a trace of the way it came. The streamer's cut (or its draw order) depends
very slightly on history. Not chased further; worth knowing if anyone relies on
byte-exact frames across paths.

### Map speed, provisional

A 1,356 x 678 picture on four worker threads: 293 to 308 ms at the whole planet,
395 ms zoomed in, on the last linked build with other agents running. The window
keeps drawing the last picture, moved to the live view, meanwhile.

### Evidence

`B5-tools-evidence/menu-layers.jpg`, `menu-roll.jpg`, from `scripts/b5-menu.play`,
which drives the real window with synthetic pointer and key events: every base
layer, zoom, pan, target, selection, lightbox, notes, repro, teleports, places, and
the Esc layers (29 asserts, 0 failed on the last linked build).
`scripts/b5-menu-scale.play` does sizes and scales. That run found two real faults,
both fixed but *pending the linker*: at 150% the window kept its old position and
hung off the screen, and on short windows Teleport scrolled out of reach (it is now
pinned at the foot of the column).

## Verified, and how

*To be finished when the linker is back.*

## Not verified

Nothing here has run in a real window: the brief bans it, and these need the lead.

- T, P, Shift+P, M, Shift+M and Esc through `App::window_event`, and
  `ModifiersChanged`. `route_key` and the window code are tested; the wiring between
  them is not.
- Letting go of the mouse on open and taking it back on close.
- P's hold in the app: the banner, the frames counted, and that the app's clean
  frame matches the harness's (`photo` uses the same rule, and sets the observer as
  the app does, but the two have never been compared).
- Shift+P's read of the swapchain frame, and how long that stutters.
- Frame cost in the app with the window shut: `--bench` doesn't go through the app.
  By reading: `Menu::ui` returns on its first line, plus a few `Option` checks.
- A real mouse wheel: the smoothing, and how fast zoom feels.
- A real OS display scale: 150% here is egui's pixels-per-point in a synthetic pass.
  Changing scale while running, and dragging the window.
- Whether the colours, the legend, and how the map looks suit Andrew. His to judge.

## Proposed wording for the lead

For `DESIGN.md` or `STATUS.md`, if wanted: "P takes a photo and T opens the map,
photos and teleport window; the scenic places moved there. M colours the mesh by
region, Shift+M adds lines. Photos live in `game/interchange/`, which git ignores."
