# Triangulum engine (`game/`)

The new engine: a planet-sized cube-sphere voxel world you can walk on, dig
into, build on and come back to. This is the M2a state: walk, fly, dig,
build, save, the July sky and lighting (M1c), and the first simulation:
sand and gravel fall.

## Running

You need Rust (stable, MSVC on Windows) and the baked planet. The bake is
not in git: either run `python scripts/bake_world.py` from the repo root
(about 20 s; it needs `output/seed42_r8` from planetgen) or copy
`game/assets/world/seed42/` from a machine that has it. Copy, never symlink.

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
| left mouse | break the targeted block (hold to repeat) |
| right mouse | place the selected block on the targeted face |
| 1–9 | select a hotbar block |
| scroll | fly speed |
| R | drop to the ground here, in walk mode |
| P | next preset place (hills, mountains, highest peak, steep coast, orbit) |
| F | hide / show water |
| H | stats panel on / off |
| V | vsync on / off |
| [ / ] | slow / speed up time (0, 0.1, 1, 10, 60, 600, 3600×) |
| Q | save and quit |

The HUD shows the targeted block (with an outline around it), the hotbar,
and in the stats panel the frame times, the streaming state, your position
and how many chunks are edited and unsaved.

The player is a 0.6 × 1.8 m box, eyes at 1.62 m, reach 6 m, with radial
gravity, a 1.28 m jump and simple buoyancy in water. There is no fall damage.

## Useful options

```
--seed N --block-size S --height H --sea-level K --n N --bake DIR --vscale F --roughness F
                      world parameters (defaults in assets/world.json)
--save DIR            save directory (default saves/<world name>)
--no-save             neither load nor write a save
--sim-radius M        active region of the simulation around the player (112)
--no-sim              start with the simulation off
--hotbar a,b,c        blocks on keys 1-9 by name (default: the first nine solid blocks)
--walk | --fly        starting mode
--hide-water          start with water hidden
--lod-factor F        LOD aggressiveness (5)
--size WxH --no-vsync --backend vulkan|dx12
--screenshot PATH --exit-after SECS     save a frame with the HUD and quit
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
`fog_per_radius` expose those optical lengths for later look tuning.
The moon's orbit supplies light only; a visible moon, eclipses, clouds
and weather remain M5 work. Cube face normals and per-vertex AO stay in
the existing packed quad format; continuous natural-surface normals and
cave skylight propagation need additional terrain data.

For repeatable stills, use `--time 17:30 --time-scale 0`. Existing
`--bench`, `--shots` and `--play` renders hold the initial sky time while
streaming/reading back frames; the play harness does not yet advance the
sky on its simulation steps. A dedicated settled evidence reel seeks the
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

## Saves

Only chunks that changed are saved, whatever changed them (you or the
simulation), under `game/saves/<world>/` (gitignored):

```
header.json      format version (3), generator version, body id, the
                 world's identity (seed, block size, H, sea level, N,
                 vertical scale, roughness), the projection and its table's
                 SHA-256, the bake's content SHA-256, the block-id table
commit.json      the current generation: which file holds each chunk, the
                 simulation state file, and a SHA-256 for every one of them
chunks/*.gN.chunk  one LOD-0 chunk each, in full (ids and water levels),
                 written in generation N
sim.gN.bin       the simulation: tick, update queue, frozen updates, blocks
                 in flight
```

A save is one snapshot: a generation's files are written and synced first
and `commit.json` is switched last by rename, so a crash mid-save leaves
the previous generation whole (files the commit does not name are
leftovers and are ignored, then removed by the next save). Every file is
checked against its digest, every chunk against the world's bounds and the
block table, before anything is installed. Dirty chunks are acknowledged
only once their generation is committed, so a failed write is retried.

Edits are saved a couple of seconds after you stop editing (at the latest
15 s after the first unsaved edit) and when you quit. A save whose header
does not match the running world is refused at startup with the reasons;
start with the saved settings or point `--save` somewhere else. Saves from
before M2a (format 2) are refused the same way. Edited chunks are
authoritative: they are never regenerated, so changing the generator around
them only changes the terrain around them. Hiding water (F) is a view
setting and never reaches generation, so an edit made with water hidden
keeps the water in the saved chunk.

The bake's identity is a SHA-256 of its rasters' contents, cached in
`content.sha256.json` beside the bake and recomputed (a few seconds) when a
raster's size or time changes.

Coarse LODs rebuild from edited data: a coarse voxel shows the topmost
solid block in it when that block was placed by an edit (a one-block bridge
stays visible from kilometres away), becomes air when the whole voxel was
dug out, and otherwise keeps the generator's value.

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
`saves/_play/<script>/`, wiped at the start unless `--keep-save`.

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

The command list lives in the header of `src/play.rs`; the ones you will
use most:

```
atblock FACE I J K       stand on a lattice block (fly mode: hover there)
at LAT LON [ALT]         teleport by coordinates
lookaxis +i -30          face exactly along a lattice axis, pitch in degrees
aim DI DJ DK             look at a lattice point relative to the feet cell
hold w,shift 2           hold keys for seconds; tap break|place|jump|g|1..9
tick N | sim on|off      N simulation ticks (20 Hz); pause or resume it
shot NAME                settled frame + state; bench N; state NAME
save | reload            write edits; quit-and-restart from disk
assert FIELD OP VALUE    position, altitude, on_ground, in_water, crosshair,
                         block[F,I,J,K], block_rel[DI,DJ,DK], falling,
                         falling_rel[DI,DJ], queued, sim_hash,
                         cover[F,I,J,K] / cover_rel[DI,DJ,DK] (what the
                         streamer draws at a block, after a shot), ...
```

Writing a script: teleport with `atblock` rather than walking to places,
probe the terrain with `column DI DJ` first, and target blocks with `aim`
(or `lookaxis` with a computed pitch) so a run is deterministic.

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
picking across a face edge), and the simulation: neighbour
notification across chunk and face edges, the chain collapse and the drain
order, settling above a block built into a fall, frozen updates waiting and
persisting, and the determinism hash across two runs, rayon pools of 1 and
8 threads, and a save and reload mid-fall. Several need the bake at
`C:/code/triangulum/game/assets/world/seed42`. The 5,041-block stress test
is `cargo test --release -- --ignored stress`.

## Known gaps (M2a)

- No water simulation yet (M2b); water is still full or absent, though the
  store and the saves now carry a level per block.
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
