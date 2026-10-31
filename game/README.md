# Triangulum engine (`game/`)

The new engine: a planet-sized cube-sphere voxel world you can walk on, dig
into, build on and come back to. This is the M2b state: walk, fly, dig,
build, save, the July sky and lighting (M1c), planetgen's lakes and rivers
as water at rest (M2-hydro), and the simulation: sand and gravel fall
(M2a), water flows by Andrew's 256-level rule (M2b), Andrew's look
panel (M3a), and new blocks with textures painted in the game (M3b).

## For Andrew: the look panel

Press **F1** in the game. A panel opens on the right and the mouse is
yours to use it (F1 again closes it and gives the mouse back to the game).
You can keep walking or flying with the keys while it is open, and
everything you change shows straight away.

- **Sky and light**: the time of day and how fast it goes; how high the
  air goes (raise *Atmosphere height* when the sky turns black on a
  mountain); the fog; the colours of the sky low down, overhead and around
  a setting sun; the stars; the sun; the light in shadow; how bright the
  night is and the moonlight; and how bright the sides and undersides of
  blocks are.
- **Water**: its colour, how see-through it is, the sun's sparkle on it,
  and the tint when your eyes are under water. *Far ocean follows the
  water colour* keeps the sea seen from high up matching.
- **Blocks**: every block's colour. Tick *faces* to give a block its own
  top, sides and bottom. Grass is green on top and dirt on the sides and
  bottom, so a tunnel under grass has a dirt ceiling. Here you also add
  blocks, choose what keys 1 to 9 hold, and paint textures (below).
- **Distance detail**: how much detail far-away land keeps. Higher looks
  better and costs speed.
- **Terrain**: roughness, landforms and how tall the mountains are
  (vertical scale 1/70, 1/20, 1/8). These rebuild the world, so the game
  shows you a *preview world* with saving turned off: your own world and
  its save are never touched. *Back to my world* takes you home. *Make
  this the default...* makes new worlds start with this terrain, but saves
  made with the old terrain will not load any more, so only do it when you
  are sure.

Sky and light, Water, Blocks and Distance detail each have a reset button
that puts that section back to the look the game ships with (each block
also has its own small reset); in Terrain, *Back to my world* does that.

**Saving and sharing.** *Save look* writes your choices into two files:
`game/assets/look.json` (sky, light, water, detail) and
`game/assets/blocks.json` (block colours). *Revert to saved* throws away
what you changed since. To share a look, commit and push those two files,
or just send them: everyone who pulls gets your look. You can also edit
the files in a text editor; the game notices within half a second.

**Screenshots.** *Screenshot* saves exactly what you see, without the
panel, to `game/out/look/`, with a `.json` file beside it that lists every
look value, the time of day and where you stood. Send both, and whoever
gets them sees what you saw and can rebuild your look.

### Adding a block

1. Press **F1**, open **Blocks**, and click **Add a block...**.
2. Type a name ("red brick"; the file calls it `red_brick`), pick its
   colour, and choose **solid** (stays where you put it) or **falls like
   sand**. Click **Add block**.
3. The new block is at the bottom of the list, opened, ready to paint.
   Under *Hotbar*, pick a key and click **Put it there**. Close the panel
   (F1), press that key, and right-click to build with it.

The block is written to `blocks.json` straight away, so a world that holds
it always finds it. Blocks are never deleted (your world may hold some):
tick **hidden** to take one off the hotbar list instead. Its colours and
texture are kept with *Save look*, like every other block's.

**Changing a block.** Click its name in the list. You can rename the blocks
you added (the old name is kept in the file, so saves and replays that know
it still work); the blocks the game ships with keep their names, because
the world generator finds them by name. You can switch a block between
*solid* and *falls like sand*: the panel tells you what that does first (it
changes the game's rules: your world keeps working, blocks of that kind
already in the world fall only when something next to them changes, and
replays recorded before will show a difference).

Every block you add is solid. See-through blocks (glass, leaves) and new
liquids aren't in the game yet.

**The hotbar.** *Hotbar: what keys 1 to 9 hold* has a box per key. Choose a
block for each; *Back to the default hotbar* puts the shipped one back.
It is saved in `blocks.json` at once and comes back next time you play.

### Painting a texture

Click a block's name, and under **Texture**:

- **Paint** chooses which faces you paint: *every face*, or just the *top*,
  the *sides* or the *bottom*.
- **Start from the block's colour** gives you a 16 x 16 picture of the
  block's colour to paint on. **Import PNG...** takes any PNG picture (a
  photo, a drawing): it is cut to a square and made 16 x 16. You can also
  drop a PNG file on the game window.
- Paint with **Pencil** (click or drag), **Fill** (fills the patch of one
  colour you click) and **Pick colour** (takes a pixel's colour; a right
  click does that with any tool). Click the colour box to choose a colour;
  colours you used appear beside it. **Undo** (or Ctrl+Z) takes back a
  change.
- Every change shows on the blocks in the world at once, and on the
  hotbar.
- **Try an example**: three placeholder pictures (stone, dirt, grass top)
  show what a texture looks like; one click puts one on the faces you
  chose. Painting on an example makes your own copy, so the example stays
  as it is.
- **Remove texture** puts those faces back to their colour.

Far away a texture shows its average colour, so a textured block looks the
same from a distance as a flat one of that colour.

**Saving and sharing textures.** *Save look* writes your pictures as PNG
files into `game/assets/textures/` (named after the block, like
`red_brick.png`) and puts their names in `blocks.json`. To share, commit and
push `blocks.json`, `look.json` and the `textures` folder, or send them.
You can also paint a texture in any paint program: open its PNG from
`game/assets/textures/`, save it, and the game shows the change within a
second while it runs.

**Texture size** is one setting, `"texture_size": 16` near the top of
`blocks.json`. 32 gives finer pictures; that choice is yours to make (it is
on the list of open decisions). Pictures of another size are resized when
the game reads them, so changing it later keeps your textures.

## Running

You need Rust (stable, MSVC on Windows) and the baked planet. The bake is
not in git: either run `python scripts/bake_world.py` and then
`python scripts/bake_rivers_v2.py` from the repo root (about 20 s and 60 s;
they need `output/seed42_r8` from planetgen) or copy
`game/assets/world/seed42/` from a machine that has it. Copy, never symlink.
Without `hydro.bin` (the second script's output) the world has no lakes or
rivers, only the sea and dry valleys.

```
cd game
cargo run --release                  # walk on the hills (first build: a few minutes)
cargo run --release -- --help        # every option
```

The engine runs from any current directory; a relative `"bake"` in
`assets/world.json` is resolved against `game/`.

Start somewhere else:

```
cargo run --release -- --at 12.5 -40.2 300         # lat, lon, altitude (m above sea)
cargo run --release -- --at 12.5 -40.2 300 --look 45 -20
cargo run --release -- --find-places               # the presets and their lattice coordinates
```

You start in walk mode when you begin within 50 m of the ground and in fly
mode when higher; `--walk` and `--fly` force it.

## Controls

| Key | Does |
|---|---|
| click | capture the mouse (Esc releases it) |
| mouse | look |
| W A S D | walk (fly: move along the view) |
| Space | jump; swim up; fly up |
| Ctrl | sneak: slow, and never off an edge (build a bridge by backing up); swim down; fly down |
| Shift | sprint (fly: 4× speed) |
| G | toggle walk / fly |
| left mouse | break the targeted block (hold to repeat); holding water, remove the targeted water cell |
| right mouse | place the selected block on the targeted face; holding water, place a full cell of water |
| 1–9 | select a hotbar block (9 is water: the bucket) |
| scroll | fly speed |
| R | drop to the ground here, in walk mode |
| P | next preset place (hills, mountains, highest peak, steep coast, orbit) |
| F | hide / show water |
| H | stats panel on / off |
| V | vsync on / off |
| [ / ] | slow / speed up time (0, 0.1, 1, 10, 60, 600, 3600×) |
| F1 | the look panel (frees the mouse while open) |
| F10 | start / stop recording a replay (below) |
| Q | save and quit |

The HUD shows the targeted block (with an outline around it), the hotbar,
and in the stats panel the frame times, the streaming state, your position
and how many chunks are edited and unsaved.

The player is a 0.6 × 1.8 m box, eyes at 1.62 m, reach 6 m, with radial
gravity, a 1.28 m jump and simple buoyancy in water. There is no fall damage.

## Reporting a bug: press F10 (M2c)

When something looks or behaves wrong, record it:

1. Press **F10**. The window title says `REC replay`, and the stats panel
   (H) names the file. (It saves your game first.)
2. Do the thing again, or just keep playing until it happens.
3. Press **F10** again to stop. Quitting with Q or closing the window
   also stops it, and if the game crashes the file keeps everything up to
   the last second.
4. Send the file. It is in **`game/replays/`**, named after the world and
   the time, for example `game/replays/Neisor-20261025-154807.replay`.
   Attach it to a message or drop it into the shared folder with a line
   about what went wrong and roughly when ("about 20 s in, the water
   climbed the wall"). The file is the whole bug: it holds the world as it
   was when you pressed F10 and every key, click and mouse move after it.
   It does not hold your save, so you keep playing as usual.

A replay usually takes a few kilobytes per minute plus the edited part of
your world (a few kilobytes per chunk you changed).

Playing one back (what an agent does with your file):

```
cargo run --release -- --replay game/replays/Neisor-20261025-154807.replay
cargo run --release -- --replay FILE --shot-at 600 --shot-at end   # settled shots at steps (60 a second)
cargo run --release -- --replay FILE --watch                       # in the window, at real time
```

It plays in a scratch save, `game/saves/_replay/<name>/`, never in your
own, checks the world, the player, the simulation and the sky against
the recording every second, and says whether it reproduced exactly or
where it first went different, and how ("step 1260, 21 s in: player
position 0.4 m apart; world: face 2 chunk (3349, 1797, 17) differs").
The shots and a `report.json` go to `game/out/replay/<name>/`. The exit
code is 0 when it reproduced, 1 when it diverged, 2 when it could not
play (the wrong bake, a damaged file). A replay needs the same world
settings and bake it was recorded with, and is meant for the same build
of the game; it says so when it cannot play.

## Useful options

```
--seed N --block-size S --height H --sea-level K --n N --bake DIR --vscale F --roughness F
                      world parameters (defaults in assets/world.json)
--save DIR            save directory (default saves/<world name>)
--no-save             neither load nor write a save
--sim-radius M        active region of the simulation around the player (112)
--no-sim              start with the simulation off
--water-period N      ticks per water step (1: a cell per 20 Hz tick)
--no-slide            the rules before B4: straight down only, and the film stays (a later --film overrides that part)
--film MODE           what becomes of level-1 water: keep, seek, merge, dry, seek+dry (the default)
--film-reach N        how many cells a film runs along a dry line to a drop (8)
--film-dry-s S        a film dries S to 3 S seconds after it comes to rest (10)
--hotbar a,b,c        blocks on keys 1-9 by name (default: the first eight solid blocks and water)
--walk | --fly        starting mode
--hide-water          start with water hidden
--lod-factor F        LOD aggressiveness (look.json's detail.lod_factor, 5)
--look-file FILE      the look to start from and save to (assets/look.json)
--panel               start with the look panel open
--size WxH --no-vsync --backend vulkan|dx12
--screenshot PATH --exit-after SECS     save a frame with the HUD (and a
                      PATH.json sidecar with the look) and quit
--time HH:MM          start at local solar time at the starting longitude (default 12:00)
--time-scale F        clock speed, 0 pauses (default 1; maximum 3600)
--day-length SECS     seconds per game day (1440 = 24 real minutes)
--lunar-month-days N --months-per-season N --year-days N
                      July calendar parameters (7, 3, 84)
```

Block size, height, sea level and N are validated at startup; an
unsupported combination (for example a sea level that is not a multiple of
H / 8, or an N the coarsest chunks do not tile) is refused with a message
saying what to change.

## Sky and time

The sun, moonlight, stars and season phase all read one absolute clock.
`--time` also accepts decimal hours (`--time 17.5`). It seeks the ephemeris
to that solar hour at the initial location; flying to another longitude
does not move the sun. `[` and `]` change speed without jumping the time;
the window title shows the chosen speed. At 1× a game day is 1,440 seconds.
The app advances the clock on the same fixed steps as player physics.

The July calendar is provisional until Andrew confirms it. Its settings
live in the `sky` object in `assets/world.json`, with the CLI overrides
above. A year must equal four seasons × months per season × lunar days;
for example, use `--lunar-month-days 8 --year-days 96` together. The
season phase is the orbital mean anomaly, including July's 0.45 epoch.
Time starts fresh on launch; it is not yet part of the chunk save format.

The gradient, sunset colors, sun disc/halo, star colors, sun/sky lighting,
moonlight levels and underwater tint retain July's equations. Atmosphere
height, limb thickness and fog distance scale with planet radius. The
`sky` JSON fields `atmosphere_height_radii`, `limb_height_radii` and
`fog_per_radius` set those optical lengths' defaults; since M3a the look
(`assets/look.json`, below) carries the atmosphere height and the fog in
metres and overrides them, and the limb keeps its July proportion to the
atmosphere. The moon's orbit supplies light only; a visible moon, eclipses, clouds
and weather remain M5 work. Cube face normals and per-vertex AO stay in
the existing packed quad format; continuous natural-surface normals and
cave skylight propagation need additional terrain data.

For repeatable stills, use `--time 17:30 --time-scale 0`. Existing
`--bench`, `--shots` and `--play` renders hold the initial sky time while
streaming/reading back frames; the play harness advances the sky on its
simulation steps only when a script asks (`skyspeed`, see Video below), and
`skytime` / `sun` seek it. Since M2c the clock belongs to the session and
moves on its 60 Hz steps in the game, the harness and a replay alike. A dedicated settled evidence reel seeks the
same clock explicitly for noon, golden hour (+5° sun), dusk (−3°) and night:

```
cargo run --release -- --sky-reel --bake C:/code/triangulum/game/assets/world/seed42 --out out/sky
```

It writes sixteen 1080p sky PNGs, two underwater diagnostics, a JSON
manifest with camera/time/sun values, and `july-comparison.play` for the
frozen viewer. Engine ground eyes are 1.7 m high; July comparison eyes
are 50 m above ground to clear its foliage. High views match altitude /
planet radius. Each pose must settle successfully. The underwater pair
also checks liquid occupancy and byte-identical paused captures.
`--time` sets the starting time for ordinary bench poses;
the reel selects its own four times. Reversed-Z, the infinite far plane
and camera-relative vertex positions are preserved.

## The look (M3a)

Andrew's dials live in three files, all tracked in git:

- `assets/look.json` (`src/look.rs`): the render look in plain units:
  `sky` (atmosphere height in metres, fog per km and its strength, horizon
  / overhead / sunset colours, sunset strength, night sky, stars), `light`
  (sun, sky ambient, light in shadow, extra night light, moonlight and its
  colour, side / underside shades, corner shadows), `water` (opacity, edge
  opacity, glint and its sharpness, underwater tint, `far_ocean_follows_water`)
  and `detail.lod_factor`. Missing fields take the default, which is the M1
  look (July's constants, the optical lengths from `world.json`'s `sky`):
  with the shipped file the M3a renderer draws the M2b frames to within
  2/255 (`scripts/m3a-look.play` run on both engines with the M2b
  blocks.json). Unknown fields and out-of-range values are refused by name.
  Every renderer (app, bench, harness, reels) starts from the installed
  look; `--look-file` names another.
- `assets/blocks.json`: block colours. `color` is every face unless the
  block gives `top`, `side` or `bottom` its own. The GPU colour table has
  three entries per id (`id * 3 + face`) and meshes carry ids, so a colour
  change rewrites a few hundred bytes and remeshes nothing. The panel
  writes the file back in its own style (one aligned line per block, ids
  and every other field kept); `blocks::colours_into_text` reproduces the
  shipped file byte for byte.
- `assets/world.json`: the terrain dials (`roughness`, `landform`,
  `vertical_scale`), written only by the panel's *Make this the default*,
  which changes the three values in place and nothing else.

`src/workshop.rs` is the panel. Per frame it applies changes (the look into
the sky uniform, colours into the colour table, the LOD factor into the
streamer) and polls the three files twice a second; with the panel closed
that is a few comparisons and three metadata reads a second. A changed file
applies at once: look.json whole, blocks.json the colours of the blocks
whose colours changed in it (new blocks or flags need a restart), and a
change to world.json's terrain dials starts a preview. Terrain dials build
a *preview world*: a new generator and streamer over an empty edit store,
with the save detached (`Session::save = None`) and the real engine, save
and simulation set aside untouched; anything unsaved in the real world is
flushed first, and when nothing is, the save's files are not touched at
all. *Back to my world* (or the dials returning to the save's values)
restores them. The *Screenshot* button reads back the next frame, drawn
without the panel, into `out/look/look-<UTC time>.png` with a JSON sidecar
(the tidied look, every block's face colours, terrain, time, and the
session state); `--screenshot` writes the same sidecar.

## Blocks and textures (M3b)

The registry (`src/world/blocks.rs`) changes while the game runs. It holds
its definitions behind a lock and the four flags the hot loops read
(solid, liquid, gravity, replaceable) as one relaxed atomic load each, with
room for every id, so the world, the mesher threads, the simulation and the
player see a new block or a changed flag at once, and an id past the end
reads as air-like instead of panicking. `blocks.json` gains:

- `texture` (every face), `top_texture`, `side_texture`, `bottom_texture`:
  PNG files relative to the `textures/` folder beside blocks.json (no `..`,
  no drive); `texture_size` at the top (16; a power of two, 4 to 256).
- `hidden` (off the hotbar list; blocks are never deleted), `was` (former
  names, oldest first) and `hotbar` at the top (nine block names).

Ids are append-only. A rename keeps the old name in `was`, and
`Registry::id` answers to former names. The panel writes structural
changes (a new block, a rename, falling, hiding, the hotbar) at once with
`blocks::save_defs`, which updates names and flags and appends whole
entries, leaving colours and textures on disk alone; *Save look* writes
colours and textures (`blocks::save_look`) after writing the painted PNGs.
Hot reload merges the file into the running registry
(`Registry::merge_file`): new blocks, renames that list the old name in
`was`, falling, hiding, the hotbar and the texture size apply live; a file
that renumbers or drops a block, or changes solid, liquid or replaceable, is
refused with the reason.

**Saves and replays across a new block.** A save's header lists (id, name)
for the blocks it knew; the check accepts a registry that extends it, and a
saved name may be a block's current name or one in its `was`
(`world/save.rs`, the header check and `extend_header_blocks`, which adds
blocks appended since to the header on the next save, so every id a chunk
can hold has its name there). The simulation's rules digest hashes each
block's first name and flags; `Sim::state_hash` covers the rules of the
blocks the state can hold (up to the highest id the generator places, an
edited chunk holds or a falling block is), so appending a block changes no
hash and a replay recorded before plays back exactly
(`src/compat_tests.rs`). Changing whether a block falls does change the
hash: replays recorded before report a difference, as they should. The
save's simulation note compares against every prefix of the registry
(`sim::rules_extend`). The game, not the harness, uses the file's hotbar
(`--play` and replays keep theirs); changing the hotbar, or a block's
falling, ends a replay recording first, as a terrain preview does.

**Textures** (`src/render/textures.rs`): a `Picture` is size x size RGBA8,
sRGB as in the PNG, made opaque (blocks are solid). Any PNG is cut to its
middle square and resampled to `texture_size`: repeated pixel for pixel
when smaller, area-averaged in linear light when larger. `TextureSet` maps
files to layers of one `Rgba8UnormSrgb` texture array, keeps each layer's
average colour, and polls the files round robin (four a frame) for hot
reload; a missing file leaves its faces flat and is noticed when it
appears. The renderer uploads every mip level, built on the CPU by 2 x 2
averages in linear light, so the 1 x 1 level is the average colour; the
sampler is nearest up close and linear between mip levels. The colour
table's alpha is the layer a face samples, or -1 for its flat colour, and
the colour is then the texture's average (the HUD swatch). The shader
(`render/shader.wgsl`) computes texture coordinates in blocks along the
face, so a texture tiles once per block on a greedy quad and on a coarse
voxel alike, and every face reads unmirrored from outside (sides with
their top up); `fs_main` takes the derivatives first and samples with
`textureSampleGrad` only for a textured face, so a flat face's colour
math is exactly the old one (frames with no textures match the pre-M3b
build bit for bit).

The panel's Blocks section is `src/workshop/blocks_panel.rs`; the pixel
editor is `src/workshop/editor.rs`. Painting writes the editor's picture
into the face's own file: a face showing an example, or a file another
block or face also uses, gets a new file named after the block (copy on
write). *Import PNG...* opens the Windows file dialog (the `rfd` crate) on
a thread; a PNG dropped on the window imports too.

`TRIANGULUM_PANEL_BLOCK=stone` (with `TRIANGULUM_PANEL_SECTION=blocks` and
`--panel`) opens a block's details for screenshots.
`game/tools/make_example_textures.py` writes the example pictures, and with
`--every-block DIR` a copy of blocks.json with every solid block textured
(each picture averaging to its face's colour) for the bench and the far
look checks. `game/tools/window.ps1` drives the real window (below).

Evidence scripts: `scripts/m3a-tunnel.play` digs a tunnel and a shaft
under grass (asserts the blocks, shoots the ceiling and walls);
`scripts/m3a-look.play` shoots the sky from 1.5 km, the hills at night,
dusk and golden hour, and the sea, for comparing look files;
`scripts/m3a-look-vscale8.play` the 1/8 highest peak. For panel
screenshots, `TRIANGULUM_PANEL_SECTION=blocks` (or sky, water, detail,
terrain) opens `--panel` on that section.

## The simulation (M2a)

A fixed 20 Hz tick runs on the main thread, every third 60 Hz player step,
so a scripted session always ticks at the same steps. It is a separate
clock from the sky's: `[` and `]` speed up the day, not physics. The tick
drains a block-update queue ordered by (due tick, position) and, through it,
the first rule: a block whose definition has `"gravity": true` (sand,
gravel in `assets/blocks.json`) and whose cell below is `"replaceable"`
(air, water) leaves the grid, falls straight down its own column as a
moving block under the player's gravity, and lands in the first cell above
something solid. Removing it notifies the block above, which falls a tick
later, so digging under a column collapses it in a chain and the column
lands in order. Every block change, by you or by the simulation, notifies
its six neighbours across chunk borders and cube-face edges.

**Sliding (B4).** A resting block with a free cell beside it and a free cell
below that one moves over and falls down that column. So powder cannot stand
in towers or cliffs: it comes to rest with neighbouring columns at most one
block apart, a slope of one block per block along the grid (45 degrees, and
about 55 down the diagonal of a pile, which comes out diamond-shaped). A pit
dug in a beach slumps in. A slide needs a free cell of its own below the free
cell, counting the blocks already in flight in that column, so every slide
ends at least one layer lower and a heap cannot cycle; a column takes blocks
as fast as they land, so a cliff pours rather than bursts. The way a block
goes is a pure function of the two cells and the tick. It works across chunk
borders, cube edges and cube corners, and a block whose way lies through a
chunk outside the active region waits where it is, with its update saved,
until the region arrives. Natural sand and gravel do not move until a change
reaches them: a generated beach steeper than the slope stands until touched.
`--no-slide` is the old rule, unit for unit.

Only LOD-0 chunks within `--sim-radius` metres of you (112 by default) are
simulated; elsewhere the world is frozen. An update due in a frozen chunk
waits until you come back, and is saved meanwhile; a block falling there
hangs mid-air until then. Blocks in flight are drawn as cubes and saved
with the queue, so a quit mid-collapse resumes mid-collapse. Snow does not
fall (a flag away if Andrew wants it); nothing but sand and gravel does, and
stone never collapses (Andrew's call).

The stats panel shows the tick, blocks in flight, queued and frozen updates,
active chunks and the tick cost. Idle ticks cost microseconds; a 5,041-block
slab in flight costs about a millisecond a tick, with the mass start (68 ms)
and the landing ticks (5-11 ms) as the known spikes.

## Lakes and rivers (M2-hydro)

The generator puts planetgen's lakes and rivers into the world as water at
rest (`src/worldgen/hydro.rs`; the bake is `scripts/bake_rivers_v2.py`,
which writes `hydro.bin` and `hydro.json` beside the rasters). Every column
may have a water surface; cells below it that are not solid hold water, with
the exact level (1-255 of a block) at LOD 0 and whole voxels further out:

- **Lakes** (771 on Neisor) are flat at their level, with a sandy beach and a
  low apron where the land around dips below the water. A lake's level never
  stands more than 25 m (Earth scale) above the bake's own shore.
- **Rivers** (20,504 links of the drainage graph, every one carrying more
  than 120 m³/s) run in meandering channels 4-32 m wide, in a valley a metre
  above the water. Gentle reaches are a smooth surface at rest (it leans at
  most 0.6 levels per block); steeper ones are pools separated by cascades
  of up to a block, and drops of 3 blocks or more are waterfalls (up to 24
  blocks each) in a short gorge, with a plunge pool and a one-voxel water
  curtain. Rivers meet the sea at sea level and lakes at the lake's level.
- **No water walls:** water is always held by ground at least 0.6 blocks
  above it, or by water within one level of it, except where a river drops
  on purpose (its steps and falls). The gates are `cargo test --release
  water_is_contained` (lake shores and 140 km of rivers) and
  `water_agrees_across_lods`; `-- --ignored water_is_contained_wide` sweeps
  a tenth of the whole network (about 1,000 km, a few minutes).

Partial levels are drawn since M2b (sloped surfaces, below), and the
levels are what the water rule reads and writes.

Evidence shots of a lake shore, a river reach, a waterfall, a river mouth
and a river crossing a cube edge, from the ground and from 300 m:
`python scripts/hydro_shots.py` (frames in `out/play/<site>/`).

## Water (M2b)

Andrew's rule (`DESIGN.md`, "Water"; the code and the settling argument are
in `src/sim/water.rs`). Every cell holds one byte of water level, 0 to 255
(a cell is water exactly when its level is at least 1); the generator's
seas are full cells and its lakes and rivers carry their own levels. Each
tick, every scheduled air or water cell:

1. **falls**: water moves straight down into the cell below while that cell
   is air or water with room, as much as fits (a free-standing column drops
   one cell per tick as a unit);
2. **spreads**: what is left levels with its lateral neighbours that are at
   least 2 levels lower (water-filling: the lowest neighbours are included
   while they sit at or below the running mean, every included cell gets the
   floor of the mean and the remainder goes one unit each to the lowest).

So water only moves downhill, a cell with room below it never spreads, and
water that nothing moves differs from every lateral neighbour by at most 1:
resting water may lean 1/256 per cell, and does: a pit dug beside the sea
fills and leaves a dish one level per metre deep around it (18 levels, 7 cm,
at the pit, flat again 19 m out). A lake drained through a channel keeps
everything below the channel's sill. The rule provably settles: the potential
`sum over cells of (1024 k + L) L` strictly falls on every tick in which
anything moves (the tests check it), so there is no cycle and no squirming.
Thin films creep: a cell moves one unit a tick when it is 2 above its
neighbour, so the last levels of a drained lake take hundreds of ticks.

**Water off a ledge (B4).** A cell that can neither fall nor spread, beside a
free cell whose floor has room, moves half its level straight into the cell
below that free cell: the level-1 cell at a ledge, which the 2-level rule can
never move, goes over. In the potential this is exactly a fall, so the reason
water comes to rest is unchanged.

**The film (B4).** On its own the ledge rule only lowers what a drained area
keeps by one level: a cell at level 1 beside a dry one is stuck again, so a
terrace keeps a wedge rising one level per cell from its edge. `--film`
chooses what happens to level-1 water:

| `--film` | what it does | water |
|---|---|---|
| `keep` | nothing more: the wedge and the film stay | kept |
| `seek` | a level-1 cell at the edge of the wet area looks along four straight lines, through dry cells, up to `--film-reach` cells, and runs to the nearest drop | kept |
| `merge` | nothing is wetted with fewer than 2 units, and a level-1 cell gives its unit to its wettest neighbour: water beads up | kept |
| `dry` | a level-1 cell dries after `--film-dry-s` to three times that | counted as `water_dried` |
| `seek+dry` (default) | it runs to a drop if it can, and dries if it cannot | counted |

A film never dries from water that is not a puddle: if the thicker water it
leans on reaches generated water at the region's edge, or runs past 256 cells,
it stays. So a poured block dries away, a lake keeps its shallows, and a
trickle fed from the boundary comes to rest instead of drawing water in for
ever. Cells waiting to dry cost nothing until their time comes, and they are
saved. The stats line shows `drying` and `dried`; the water balance counts it.

Updates run in a deterministic order (the M2a queue) through a tick-local
overlay and are committed in one batch per tick; a cell a neighbour spread
into waits for the next tick (water moves one cell sideways per tick), the
spread pass alternates direction every tick, and every written cell is
scheduled again for the next tick. Only cells whose level changed, plus
their neighbours, are scheduled, so settled water costs nothing: an
untouched sea processes no updates at all, and the simulation runs only
where something changed (the state hash and the tests check it).
`--water-period N` makes water step every N ticks (Andrew's flow-speed dial;
1 is 20 cells per second).

**Generated rivers are held until disturbed.** A generated river is at rest
except at its designated drops (steps, falls), where under the rule every
step would drain the pool above it. Nothing schedules a generated cell
until a change reaches it, so an untouched river keeps its shape, falls
included, at no cost. A disturbance (a channel dug from the bank) wakes
cells by notification; the waking cascades upstream along the reach as
pools drain through their steps, inside the active region only, where the
generated river upstream supplies and the generated river downstream
absorbs (the boundary, below). The reach the player disturbed settles into
a trickle fed from the upstream edge; edits are authoritative, so the
generated profile does not come back.

**The boundary of the active region.** Cells in inactive chunks are frozen:
never written. Where the generator has water, a frozen cell is a reservoir
at the level it holds now (a lake stays full and refills the region; a lake
the player drained near the edge does not refill itself when the region
shifts), and generated water that is still exactly as generated and below
full (a river cell) also absorbs, down to its level. Everything else at the boundary is a wall, so a channel dug past
the edge does not drain a lake into frozen nothing. Units crossing the
boundary are counted in the stats panel (`water in`, `water out`), as are
units the player placed and removed and units displaced or lost; the water
inside the region changes by exactly that balance.

**Placing and removing.** Key 9 holds water: the pick ray then also stops at
water, so a left click removes a water cell (its units are counted) and a
right click places a full cell (topping up a partial one). Holding anything
else the ray passes through water to the floor as before. A solid block
placed or landing in water pushes the water up its column into the first
cells above with room (a full cell passes it on); water with no room above,
under a solid ceiling, is lost and counted. Swimming is level-aware: a film
is walked on, a full cell swum in, and the underwater tint needs the eye
below the surface of its cell.

**Sloped surfaces.** Water quads are 16 bytes (`mesh.rs`): a top face sits
at level / 255 inside its block with each corner averaged over the water
cells sharing that lattice corner (Minecraft's rule: weight 10 at 0.8 and
above, air counts as 0, solid is left out, a cell with water above it is
full), so neighbouring surfaces meet exactly, and a side face against air
takes the same two top corners, so falls and partial columns are watertight.
Full water is the geometry it always was.

## Saves

Only chunks that changed are saved, whatever changed them (you or the
simulation), under `game/saves/<world>/` (gitignored):

```
header.json      format version (3), generator version, body id, the
                 world's identity (seed, block size, H, sea level, N,
                 vertical scale, roughness), the projection and its table's
                 SHA-256, the bake's content SHA-256, the block-id table
                 (M3b: blocks added to blocks.json later are appended to
                 it on the next save; a renamed block answers to the old
                 name through "was")
commit.json      the current generation: which file holds each chunk, the
                 simulation state file, and a SHA-256 for every one of them
chunks/*.gN.chunk  one LOD-0 chunk each, in full (ids and water levels,
                 the levels the water rule reads and writes), written in
                 generation N
sim.gN.bin       the simulation: tick, update queue, frozen updates, blocks
                 in flight, and (since M2c) the settings it ran under,
                 where its active region was computed (a reload resumes
                 that exact region) and a digest of the block rules
```

A save is one snapshot: a generation's files are written and flushed to
the disk first, then their folders, and `commit.json` is switched last by
a write-through rename (it returns only once the rename is on the disk),
so a crash or a power cut mid-save leaves the previous generation whole
(files the commit does not name are leftovers and are ignored, then
removed by the next save). The previous generation's files are removed
only after the new commit is on the disk, and a flush that fails fails the
save (it is retried). Every file is checked against its digest, every
chunk against the world's bounds and the block table, before anything is
installed. Dirty chunks are acknowledged only once their generation is
committed, so a failed write is retried. A save folder with a
`commit.json` but no `header.json` is damaged, not empty: it is refused
with a message instead of loading as an untouched world.

Edits are saved a couple of seconds after you stop editing (at the latest
15 s after the first unsaved edit) and when you quit. A save whose header
does not match the running world is refused at startup with the reasons;
start with the saved settings or point `--save` somewhere else. Saves from
before M2a (format 2) are refused the same way. Edited chunks are
authoritative: they are never regenerated, so changing the generator around
them only changes the terrain around them. Hiding water (F) is a view
setting and never reaches generation, so an edit made with water hidden
keeps the water in the saved chunk.

The bake's identity is a SHA-256 of the bytes the game loaded: meta.json,
the six rasters in full and `hydro.bin` when the bake has one, hashed while
they are parsed (about 0.25 s more at start-up on a CPU with SHA
instructions, about 2 s without). Nothing is cached: the old
`content.sha256.json` beside the bake is no longer read or written. Saves
made before the lakes and rivers (generator version 2) are refused.

Coarse LODs rebuild from edited data: a coarse voxel shows the topmost
solid block in it when that block was placed by an edit (a one-block bridge
stays visible from kilometres away), becomes air when the whole voxel was
dug out, and otherwise keeps the generator's value.

## Driving the real window (M3b)

`game/tools/window.ps1` drives the game's window from PowerShell, for what
the harness can't reach (the panel, file dialogs, the app's hotbar):

```powershell
. game\tools\window.ps1
$g = Start-Game -Arguments '--save saves/_live --blocks out/live/blocks.json --size 1600x900' -Log out\live\game.log
Wait-GameWindow $g
Send-GameKey $g F1                  # keys by System.Windows.Forms.Keys name
Click-Game $g 820 163               # client pixels, read off a screenshot
Drag-Game $g 740 300 1220 300
Send-GameText $g 'red brick'        # into a focused text field
Scroll-Game $g 1100 600 -5
Look-Game $g 0 200                  # relative mouse motion (when captured)
Save-GameShot $g out\live\01.png  # the client area, as on screen
Stop-Game $g                        # Q (save and quit); kills only if stuck
```

`Attach-Game PID` picks up a game started by an earlier command, so a long
session can be one command per step (save the id from `Start-Game`). It
makes PowerShell DPI aware, finds the window by process id, and taps Alt
before `SetForegroundWindow` so Windows lets it take focus. Screenshots
copy the screen, so the game must be in front and nothing else on top;
never copy the whole screen (a file dialog or another window may show
private files). Point `--save`, `--blocks` and `--look-file` at scratch
copies so a session never writes the real look files.

## The bench

The project's one performance number: a fixed 60 s flight (50 m AGL at
50 m/s for 30 s over the hills, climb to 5 km, dive back), plus settled
static poses from the ground to orbit, at 1920×1080.

```
cargo run --release -- --bench --out out/run1
```

Results land in `out/run1/results.md` (and the pose PNGs beside it). The
flight measures wall-clock time, so run it alone: anything else rendering or
compiling on the machine corrupts it. `--skip-flight` and `--only pose,pose`
narrow the run.

## The play harness

A script drives the real player, world and renderer headless on a fixed
60 Hz step, so a play session is repeatable and self-checking:

```
cargo run --release -- --play scripts/m1-done.play
```

Output goes to `out/play/<script>/`: `NAME.png` settled frames with the HUD,
`NAME.json` state sidecars, and `run.log`, the transcript. The exit code is
1 when any `assert` failed. The run has its own save directory,
`saves/_play/<script>/`, wiped at the start unless `--keep-save`. A directory
you name with `--save` is never wiped unless you also pass `--reset-save`:
`--play x.play --save saves/Neisor` plays on your real save without deleting
it first (the script's own edits are still saved into it).

`scripts/m1-done.play` is the M1 "done when" scenario: walk, dig a trench
and a tunnel, build a stone bridge over the trench, walk across it, save,
reload, check every edited block, and shoot the bridge from 300 m, 550 m
and 1 km (LOD 0, 1 and 2). It must pass.

`scripts/tunnel-wall-reload.play` digs a shaft to a chunk boundary over a
buried (Enclosed) chunk, saves, reloads and asserts with `cover_rel` that
the chunk whose top the shaft exposed is drawn at LOD 0 (review 2 #7).
`scripts/water-lod-seam.play` (at `--lod-factor 0.2`) and
`scripts/water-lod-seam-high.play` (default factor, from 30 km) frame the
sea at LOD boundaries (review 2 #9).

`scripts/m2a-sand.play` is the M2a scenario: tunnel under a column, build a
sand and gravel tower on it, knock the support out and watch the chain
collapse land in order; then the same with the simulation paused, resumed,
saved and reloaded mid-fall. It must pass too.

`scripts/m2b-water.play` is the M2b scenario: pour water beside a pit and
watch it spread, slope and drop in (levels asserted tick by tick, the total
conserved, settled within a budget); dig a pit at the sea shore and open a
channel inland, and check the pit fills, the channel floods and the sea
stays full. `scripts/m2b-boundary.play` pours water into a trench with a
small active region, walks away so the trench freezes at the region's edge,
comes back and watches it finish, then digs at the shore with a region so
small the sea beyond it is the generator's reservoir. `scripts/m2b-river.play`
stands by a generated river (nothing is scheduled until it is touched),
digs a trench from the bank with the region one chunk wide, and checks the
trench fills, the river inside the chunk keeps its water, and water flows
in from the generated river upstream and out into it downstream, counted.
All three must pass. `m2a-sand.play` runs under `sim slide off`, and the beach
pits of `m2b-water.play` and `m2b-boundary.play` too, because they are about
falling straight down and about water: free-standing towers and sand walls
do not stand under the slide.

`scripts/b4-slide.play` is the slide: a tower falling apart, a dropped cube
becoming a pile (its slope printed), a sand cliff collapsing with a save and
reload in the middle, the level-1 cell at a ledge, a shaft dug in a real beach,
a heap over a cube corner, a block waiting at the edge of the active region.
`scripts/b4-film.play` drains a terraced bowl, leaves a film on a wide table,
pours a block on a table and on real ground; it asserts only what holds under
every rule, so run it with `--film MODE` and compare the frames and the
printed levels. `b4-oldsave-make.play`, run on an older build, and
`b4-oldsave-load.play`, run on this one, check that an old save loads and that
nothing in it wakes until it is touched.

`scripts/m3b-hotbar.play` (run with `--blocks scripts/m3b-look/blocks.json`)
builds a wall of the painted `clay_brick` block from the M3b session on key
1 (the `hotbar` command), checks its blocks, and shoots it with the hotbar.

`scripts/m2c-replay.play` goes to its sites with `find`, records a replay
with the same recorder F10 uses (a sand collapse, digging at a river bank,
pouring water, the sky, hiding water, the active region, fly mode, a save
and a reload), then plays the file back in a fresh session and checks it
reproduced. `scripts/m2c-video-driver.play` checks that a recorded camera
flight steps the world (sand lands, the tick and the sky clock advance, the
frames draw the falling block). Both must pass.

The command list lives in the header of `src/play.rs`; the ones you will
use most:

```
atblock FACE I J K       stand on a lattice block (fly mode: hover there)
at LAT LON [ALT]         teleport by coordinates
find flat|shore|river|lake|waterfall
                         stand at the nearest such site, deterministically,
                         looking at the water (find_* fields say where)
replay record F | stop | check F
                         record a replay (as F10), stop, play it back here
lookaxis +i -30          face exactly along a lattice axis, pitch in degrees
aim DI DJ DK             look at a lattice point relative to the feet cell
hold w,shift 2           hold keys for seconds; tap break|place|jump|g|1..9
tick N | sim on|off      N simulation ticks (20 Hz); pause or resume it
sim radius M | period N  active region radius; ticks per water step
shot NAME                settled frame + state; bench N; state NAME
save | reload            write edits; quit-and-restart from disk
assert FIELD OP VALUE    position, altitude, on_ground, in_water, crosshair,
                         block[F,I,J,K], block_rel[DI,DJ,DK], falling,
                         falling_rel[DI,DJ], queued, sim_hash,
                         level[F,I,J,K] / level_rel[DI,DJ,DK] (water 0..255),
                         water_rel[DI,DJ,DK,DI2,DJ2,DK2] (units in a box),
                         water_moved/in/out/placed/removed/displaced/lost,
                         cover[F,I,J,K] / cover_rel[DI,DJ,DK] (what the
                         streamer draws at a block, after a shot), ...
```

Writing a script: teleport with `atblock` (or `find`) rather than walking
to places, probe the terrain with `column DI DJ` first, and target blocks
with `aim` (or `lookaxis` with a computed pitch) so a run is deterministic.

## Video

The harness also records video (commands in the "video" section of the
`src/play.rs` header; the code is `src/play/video.rs`). Between `record
FILE` and `stop`, every 60 Hz step of a script is a video tick, and frames
are piped as raw RGBA into ffmpeg, which writes H.264. Nothing else is written
to disk. `fly` moves the camera smoothly through `key`s, `pan` turns the
view, `sun` and `skyspeed` set and run the sky clock, `zoom` changes the
field of view, and `caption` places caption cues. `recmode settle` settles
every frame before writing it (clean: what the world converges to, with
pop-in between frames hidden); `recmode live` streams and renders once per
step at real time, as a player sees it. ffmpeg must be on PATH, or set
`TRIANGULUM_FFMPEG`.

The M1 showcase (`docs/media/m1-showcase.mp4`) re-renders with one command
from the repo root (about 4 minutes on the 2060 machine, needs ffmpeg and ffprobe):

```
python game/scripts/showcase.py
```

It runs `scripts/m1-showcase.play` and `scripts/m1-showcase-vscale8.play` at
1080p into `game/out/showcase/clips/`, then adds title and end cards,
crossfades and captions. It writes the full-quality master
`game/out/showcase/m1-showcase-1080p.mp4`, the 720p repo copy (kept under
15 MB) and review stills in `docs/media/m1-showcase-stills/`. If the world
or the engine changes under the scripts, their asserts (the build sequence
is checked block by block, as in `m1-done.play`) or the `fly` ground check
fail the run, and `showcase.py` stops before assembling anything. Fix the
script, using `column` and `state` to probe, and run it again.
`--skip-render` re-assembles existing clips (to retime captions or cards).

## Tests

```
cargo test --release
```

The tests cover the lattice (face-edge transforms at every edge and a
corner, at LOD 0 and a coarse LOD; configuration validation), chunk
compression and its byte format, the edit store (revisions, acknowledged
saves, water levels), saves (round trip, mismatched header refused with
reasons, an interrupted save leaving the previous generation whole, damaged
and out-of-bounds files refused), edits propagating to coarse LODs and to
neighbour chunks, the streamer (the coverage tree's invariants under random
camera paths, atomic switches, an edited Enclosed chunk becoming a node, a
chunk whose padding holds an edit existing, a stale resident mesh asked for
again when its replacement is lost), the mesher's water edges meeting
across an LOD boundary, player collision edge cases (resting exactly on a
floor, walls, jumping onto a step, half-metre blocks, walking across a face
edge and round a corner, walking along a wall through a face edge, swimming,
picking across a face edge, a film walked on and a full cell swum in), and
the simulation: neighbour notification across chunk and face edges, the
chain collapse and the drain order, settling above a block built into a
fall, frozen updates waiting and persisting, and the determinism hash across
two runs, rayon pools of 1 and 8 threads, and a save and reload mid-fall and
mid-flow. Water (`src/sim/water_tests.rs`): one cell spreading to exactly 51
on five cells and a basin levelling to rest, a column falling as a unit then
spreading with the potential strictly falling every moving tick, a full
basin resting until touched and a lake draining through a channel to its
sill, displacement by a landing block and loss under a sealed lid, a pit at
the shore filling from the sea with the water accounted exactly, boundary
cells as reservoirs, drains and walls that are never written, the water
period, and the mesher's shared corners. Replays (`src/replay_tests.rs`,
driven through the app's own input code with synthetic window events):
record then replay gives the same state, the same inputs at 144 and 30 fps
give the same world, a damaged replay names its first divergence, a replay
never touches the real save, harness pokes replay exactly, and a key tap
shorter than a frame still counts. Several need the bake at
`C:/code/triangulum/game/assets/world/seed42`. The stress tests are
`cargo test --release -- --ignored stress` (5,041 sand blocks, a 10,800-block
sand collapse, the cost of the B4 rules) and
`cargo test --release -- --ignored water_stress --nocapture` (a 2,000-cell
pour and a 10,000-cell dam break, with the per-tick cost). The B4 rule table,
every film rule on every scene, is
`cargo test --release film_rules_on_the_three_scenes -- --nocapture`.

Blocks and textures (M3b): `blocks.json` with textures round-trips
(written, read back, idempotent, back to the shipped file when the
textures go), adding, renaming, falling and hiding write the file and merge
back, contradictory flags and textures outside the folder are refused, PNG
decoding, resampling (crisp up, averaged down, cut to the middle square),
mips averaging in linear light, fill, undo, a texture file changed on disk
reloading into its layer and the colour table (and a missing file loading
when it appears), painting an example making the block's own copy, Save
look writing the PNGs and blocks.json, a blocks.json edited on disk
bringing a new block in live and a renumbering one refused, the panel's
add, rename, fall and hotbar requests through a real session, and
(`src/compat_tests.rs`) a save and a replay made before a block was added
still loading and playing back exactly after it, live and after a restart,
while a block that now falls is still caught.

The look (M3a): look.json's round trip (defaults written and read back,
partial files, unknown and out-of-range fields refused, the shipped file
is the M1 look), blocks.json's round trip (the shipped file rewritten from
its own colours is byte-identical, idempotent, keeps comments and ids),
the per-face colour table (grass, dry grass and snow undersides are dirt),
world.json's terrain edit changing only its three values, the file watch,
hot reload (a colour changed in blocks.json in a text editor reaches the
GPU colour table on the next poll; a changed look.json reaches the look; a
broken file is reported and not applied), Save look writing both files,
the far ocean matching the shipped colour, and the time-of-day seek.

The water tests (`src/worldgen/hydro_tests.rs`, they need `hydro.bin`)
check every generated water cell around 1,537 lake shores and along 300
river links for walls, perched water and rest, and coarse LODs against
LOD 0; `HYDRO_VSCALE=0.125` runs them at 1/8. Ignored helpers there measure
generation cost before and after (`profile_hydro_chunks`), find the evidence
sites (`hydro_places`) and print the water model at a column
(`hydro_probe`, with `HYDRO_PROBE="face i j"`).

## Known gaps (M2b, M3a, M3b)

- Grass and dry grass have dirt sides everywhere, so the natural terrace
  steps on grassy hills now show as brown risers (dark green before), near
  and far. That is the starting point the M3a task asked for; Andrew tunes
  it in the panel (Blocks, grass, sides).
- Underground is still lit like the surface: *Light in shadow* brightens or
  darkens tunnels and the night alike. Darkness underground needs skylight
  data (M3 or later).
- Hot reload of blocks.json applies colours, textures, new blocks,
  renames (listed in `was`), falling, hiding and the hotbar live; a change
  to solid, liquid or replaceable needs a restart (the panel says so).
- Blocks are drawn solid: no see-through blocks yet (glass, leaves would
  need a cut-out drawing pass), and the water rules know one liquid, so the
  panel adds only solid blocks. Water has no texture.
- The pixel editor has no line, rectangle or mirror tools, and no redo.
- A terrain preview keeps the world's height and sea level. At 1/8 the
  deepest ocean (about 1,380 m down) reaches below the bottom of the
  default world (512 m below the sea); 1/8 is normally run with
  `--height 4096 --sea-level 1536`, a restart setting the panel does not
  expose. What the preview draws there was not checked.
- The time of day and its speed are not part of the look; a screenshot's
  sidecar records them.
- Resting water leans up to one level per cell, as the rule allows, and the
  dynamics leave the steepest lean they can: a dish around a pit, a film on
  a drained lake that slopes up from the sill. Whether that look is wanted
  is Andrew's call (`DECISIONS.md`).
- A single placed cell spreads into a film of up to 255 cells at level 1 on
  flat ground. There is no evaporation and no minimum visible level.
- Water does not push the player or falling blocks; falling blocks pass
  through water and displace it only where they land.
- Falling blocks pass through each other and through the player; a block
  that lands in your cell pushes you up.
- Chunks entirely below every surface (`Enclosed`) are never streamed
  unless an edit touches their padded voxel grid (their own blocks or the
  one-voxel halo around them, across face edges too); one that gets edited
  appears under its coarser parent and refines down to LOD 0 over the next
  frames, so a deep shaft shows its walls coarsely for a moment.
- Quads on the sea-level plane (and all water quads) have tangent edges of
  at most 256 m (`mesh::SEA_EDGE_MAX_M`), so the sea's chords sag under
  0.1 m against the sphere and the vertices of neighbouring LODs coincide
  from LOD 5 up; a seam at the LOD 5/4 boundary and finer is below a
  centimetre. Other coarse surfaces still sag (23 m mid-quad at LOD 9),
  inside their own LOD's height quantisation.
- In coarse chunks' padding, an edit at the one diagonal column of a cube
  corner (three faces meet there) is left natural.
- The corner policy for lattice coordinates past two sides of a face maps
  the missing diagonal onto the neighbouring face; collision there is
  approximate within a block of a cube corner. A box that straddles a face
  edge without its centre crossing is swept in its own face's lattice; the
  part past the edge can lean a few centimetres into a wall on the other
  face (the two lattices meet with a kink) until the centre crosses, when
  the whole box is checked in the new lattice and nudged clear.
