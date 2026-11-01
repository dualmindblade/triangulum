# Status

*Updated 2026-10-31 by Claude (Opus).*

## Where things stand

- **M1, a planet you can dig: feature-complete.** It's waiting for you two
  to play it (commands below) and watch the video
  (`docs/media/m1-showcase.mp4`).
- **M2, things fall and flow: done.** Sand and gravel fall, water flows
  under Andrew's 256-level rule, Neisor has its lakes and rivers, and
  diverting a river keeps the area wet. **F10 records a bug as a replay
  file** that plays back exactly. Astra's third review found edge cases;
  all but two minor ones are fixed.
- **The one number** (RTX 2060, 1080p, 60 s test flight, idle machine):
  **1,188 fps average, 574 fps for the worst 1% of frames, worst frame
  5.0 ms** (everything through the bug round, 2026-10-31). Through M3b it was
  1,134 / 611 / 6.0. The target is 60 fps (16.7 ms).
- **M3, Andrew's workshop: half done.** Press F1: the look panel (sky,
  light, fog, water, block colours, terrain dials) and the block and
  texture editor (add blocks, paint textures). *Save look* writes it all
  to files in git, so his look becomes everyone's. In plain words: "For
  Andrew" in `game/README.md`. Still to come: non-full blocks (slabs,
  poles, plants) and parts inside blocks.
- Analysis and plan: https://claude.ai/artifact/7q7Lk9YM4DZxCX5WfcvxPo ·
  grid comparison: https://claude.ai/artifact/7qz75zidRkhjMbVcKU3apY. The
  July docs are in `docs/archive/`; the old viewer still runs from
  `viewer/`.

## Done

- **M1-grid** (Astra, verified by Opus, merged): `crates/cubesphere` with
  six projections. The recommended one, `EqualAreaCobe`, is smooth, exactly
  equal-area (1e-6) and the squarest: 14% of the surface more than 10° off
  square. Compare all four candidates on the grid page linked above.

- **M1-inventory** (Opus): `docs/feature-inventory.md` catalogues every
  feature of the old viewer, how it worked and where it lands. Andrew's
  specs and reference images, which were only on one disk, are now in git
  under `docs/source/`.

- **M1-bake** (Opus, verified by Opus, merged): `scripts/bake_world.py`
  removes the old "texel artifacts" at the source. Cell-structure noise is
  down from 18% to 0.6%, and it carries planetgen's geology (soil depth,
  continental cores, volcanic zones, erodibility) for real strata. Run it
  with `python scripts/bake_world.py` (about 20 s); check it with
  `python scripts/bake_world_check.py`.

- **M1a** (Fable, verified by Opus, merged): the new engine in `game/`, on
  the real equal-area grid. On an idle machine the fixed 60 s test flight
  runs at **633 fps average, 307 fps for the worst 1% of frames, worst
  single frame 4.8 ms** (the 60 fps target is 16.7 ms). Strata are real
  blocks; water is translucent. World height costs nothing: 2,048 and
  4,096 draw identical chunks.

- **M1b** (Fable, verified by Opus, merged): walking, digging, building and
  saving. The scripted M1 scenario (walk, dig a trench and a tunnel, build a
  bridge, save, reload, check the blocks, view the bridge from 1 km) passes
  83 of 83 checks. Six findings from Astra's review are fixed.

- **M1c-sky** (Astra, verified by Opus, merged): the July viewer's sky,
  sun, stars, fog and lighting, matched side by side against the old viewer
  at noon, golden hour, dusk and night. 24-minute days.

- **M1-terrain** (Opus, verified by Opus, merged): real landforms: ridges,
  gullied flanks, river valleys, mesas on hard dry rock. Oceans stay blue
  from orbit, and a cube-edge seam is fixed. Before/after sheets at every
  vertical scale: `docs/missions/M1-terrain-evidence/`. Saves from before
  this change are refused (the terrain under them changed).

- **M1c-stream** (Fable, verified by Opus, merged): streaming rebuilt
  around a coverage tree that never overlaps or leaves gaps. On an idle
  machine the 60 s test flight runs at **1,447 fps average, 602 fps for the
  worst 1% of frames, worst frame 5.7 ms**; at 1/8, which ran at 14 fps,
  it's now 964 / 381 fps. Also a sneak key (Ctrl while walking).
  (Correction: I first wrote that water surfaces now meet between detail
  levels. Astra's second review showed that fix only covers vertices; the
  edges between them can still be up to 23 m apart, far out. It's on the
  hardening list.)

- **M2a** (Fable, verified by Opus, merged): the simulation. A 20 Hz tick,
  separate from the sky clock, with a strictly ordered block-update queue.
  Sand and gravel fall when you dig out what holds them up, and land
  where they should, even across a save and reload mid-fall. The same
  inputs always give the same world (checked by a state hash). Saves are
  now crash-safe: written all-or-nothing, retried on failure, validated on
  load. Saves made before this are refused.

- **M1-hardening** (Fable, verified by Opus, merged): breaking blocks and
  walking along walls across cube edges, tunnel walls that went missing
  after a reload, and the far water seams (47 m apart, now 2 m). Each
  fix has a test that fails without it.

- **M2-hydro** (Opus, verified by Opus, merged): Neisor's lakes and
  rivers as real water: 771 lakes, about 9,400 km of rivers, waterfalls
  up to 14 m (24 m at 1/8). A strict check of about 74 million water
  cells found no water walls and nothing perched. Needs the new bake
  step `python scripts/bake_rivers_v2.py` (about a minute); without it
  the game runs with no rivers. Saves from before are refused.

- **M2b** (Fable, verified by Opus, merged): flowing water under Andrew's
  rule, proven to settle (a quantity that strictly falls on every tick
  with movement). Settled water costs nothing. Partial levels are drawn as
  sloped surfaces. Water entering and leaving the loaded area is counted
  exactly. Water is on the hotbar (key 9): left click removes, right click
  pours. On a real river, a dug trench fills and the river keeps flowing
  through: about 47,000 units in and 45,000 out.

- **M2c** (Fable, verified by Opus, merged): replays. F10 starts and stops
  a recording in `game/replays/`; the file holds the world as it was and
  every key, click and mouse move after it, so the file is the bug report.
  `--replay FILE` plays it back in a scratch save (never yours) and
  reports the first moment it differs and what differs. I altered two
  recordings by hand (walk backwards instead of forwards; place instead
  of break) and each was caught within a second and named; the unaltered
  one reproduced exactly, and a real save stayed byte-identical. All
  input now reaches the game in fixed 1/60 s steps, so a session played at
  30 fps and one at 144 fps give the same world. Also three of Astra's
  review 3 findings, and a harness command that finds a shore, river,
  lake, waterfall or flat spot near you.

- **M3a** (Opus, verified by Opus, merged): Andrew's look panel. F1 opens
  it; changes show at once; *Save look* writes `game/assets/look.json`
  and `blocks.json`, and the game reloads either file within half a
  second of an edit in a text editor. Terrain dials build a preview world
  with saving off, so his own world is never touched. A *Screenshot*
  button saves what he sees plus every setting behind it. I checked it by
  writing my own look files (a 4 km atmosphere, a brighter night): the
  sky stays blue at 1.5 km up and the night ground brightens. One look
  change: grass blocks now have dirt sides, so tunnel ceilings are dirt,
  and terrace steps in grassland show brown edges (one click in the panel
  to change back). Frame rate unchanged with the panel closed.

- **M2-hardening** (Fable, verified by Opus, merged): Astra's third
  review. Water no longer stalls at the old edge of the simulated area
  after you walk on; a river crossing that edge keeps flowing; sand
  waits at the edge instead of landing outside it. Saves are written so
  a power cut can't lose one that was reported saved (by Windows' own
  rules; a real power cut wasn't tested). A damaged save is refused
  instead of loading as an empty world, and planets too big for the
  simulation are refused with a clear message. I checked both by hand,
  and that a save from the previous build still loads. Edits use about
  a quarter of the memory they did. Replays recorded before this are
  refused (the simulation's rules changed); record new ones.

- **M3b** (Opus, verified by Opus, merged): Andrew's block and texture
  editor, in the look panel's Blocks section. He can add a block (name,
  colour, solid or falls like sand), rename or hide one, and put any block
  on keys 1-9. He paints each face's 16×16 texture in a pixel editor
  (pencil, fill, pick colour, undo) or imports a PNG; every stroke shows on
  the blocks in the world at once, and *Save look* writes the pictures and
  `blocks.json`. Saves and replays made before a block was added still
  load and play. Example textures (stone, dirt, grass) are included but
  off; the default look is unchanged until he adds one. Driven in the
  real window by `game/tools/window.ps1`, a new tool that lets agents
  click and type in the game.

- **B3** (Astra, verified by Opus, merged): aiming at cube edges and corners.
  The ray from the crosshair now follows its real path through each cube
  face, so the outline, the block you break and the block you place are all
  where you aimed. Measured against a separate check of the true ray: 1,188
  wrong blocks in 13,312 test rays near edges and corners before, none after.
  Picking costs about 0.1 ms more per frame on a busy machine. Recordings made
  at a corner before this now differ at their clicks, as they should. Walking
  right at the corners is still approximate, and real mouse use there hasn't
  been tried yet.

- **B5** (Opus, verified by Opus, merged): Andrew's tools. **M** colours the
  mesh by region (detail level, chunk, cube face, quad, block, biome, tectonic
  plate, crust) and **Shift+M** adds edges or shows edges only. **T** opens the
  map (biomes, temperature, rain, elevation, with zoom, relief, rivers and
  lakes), the photo roll, the scenic places and teleport. **P** takes a photo
  (settled), **Shift+P** the live frame; they land in `game/interchange/` with
  everything needed to repeat the shot. A choice made in the T window during an
  F10 recording plays back exactly. All of this was driven by test events with
  no window on screen: the real keys, mouse and display scaling are still to
  be tried, and that's the first thing to check.

- **B1** (Opus, verified by Opus, merged): water surfaces. Water now shows
  under a block sitting over it, the V-shaped dips along lake shores are gone
  (they were a chunk-border mistake, not real terrain), and water surfaces
  meet exactly at chunk and cube-corner joins. A hatched patch on shore walls
  went too. The thin lines on the water at 041049 are a lake height step
  between detail levels, which belongs to B2's work. Waterfall lips still lean
  over the drop, on purpose.

- **B4** (Opus, verified by Opus, merged): sand and water move like Andrew
  asked. Sand and gravel slide off ledges and settle one block per block, so a
  sand tower becomes a heap and a pit in a beach fills itself in. The last
  thin level of water now goes over an edge instead of sticking, and film left
  behind runs towards a drop or dries up after a while. That is the first time
  water leaves the world on its own, so poured water on flat ground dries in
  about three minutes. `--film` compares the other rules, `--no-slide` is the
  old game; the list of what to try is at the top of
  `docs/missions/B4-flow-findings.md`. Replays recorded before this are
  refused, with a message saying why. Saves load and stay still. Nobody has
  played it yet, so the feel is Andrew's to judge.

- **B2** (Opus, verified by Opus, merged): the shores. Lakes keep their water
  from orbit (78-85% of lake columns lost it at the coarsest detail levels;
  none do now). The sea floor is the same block at every detail level, so
  the bands are gone. Coasts are one clean line with a continuous beach about
  70 m wide, built from the planet's own coastline instead of from noise, and
  the torn inland sea at lat 3, lon -15 is one body of water with its islands.
  A strict check of 50 million water cells found no water walls and nothing
  perched. Saves and replays from before this are refused, because the terrain
  changed; the build that made them still plays them. Thin lines remain where
  a lake's surface steps between detail levels: the fix has a trade-off that's
  Andrew's call.

- **B6** (Opus, verified by Opus, merged): bug reports. Press **F9** and the game
  stops on that frame: draw on it (pen, arrow, box, text), say what's wrong, and
  Enter takes you back. During an F10 recording each mark lands at that moment
  in the recording. The T window has the marks and recordings, and *Make
  report* gathers everything into one folder under `game/interchange/reports/`
  with a `report.md` that says exactly which moment and which pixels he meant.
  Replays now report the simulation and the look separately. All driven by test
  events: F9 in the real game and how the pen feels are the first things to try.

- **R4** (Astra, verified by Opus, merged): the fixes from Astra's fourth
  review. A replay file is now treated as untrusted, so a crafted one can't
  touch a real save; a failed *Add block* can't leave a save unloadable; every
  block a save uses has a recorded identity. A cut-off recording now says it's
  incomplete instead of claiming it reproduced. Bad or huge PNGs are refused
  safely, and a held key is applied at the moment it arrived, whatever the
  frame rate. Saves record the simulation's rules, so replays made before this
  need the build that made them.

- **S1** (Astra, verified by Opus, merged): saves no longer stop the game. It
  takes a quick copy of what changed and writes it in the background: in the
  test, 400 changed chunks held the game up for about 12.7 s before and about
  2.6 ms now (a busy machine, writing to the E: hard drive). "Saved" appears
  only once it really is, quitting waits for a save in progress, and a failed
  save keeps your changes and says so. The save format is unchanged. A real
  power cut still hasn't been tried.

- **B7** (Opus, verified by Opus, merged): Andrew's powder experiment, off by
  default. `--powder levels` gives sand and gravel a level, so a pile is a
  smooth slope at a real angle (`--powder-angle`, 35 degrees unless you say
  otherwise) that you can walk up. It looks very good at rest and rough while
  it moves: flat falling tiles for a second or two, and big flows cost more.
  It plays on its own save, so nothing can touch your world, and the normal game
  is unchanged. It's for Andrew to judge, not a replacement yet. What to try
  is at the top of `docs/missions/B7-powder-findings.md`; start with
  `compare-tower.jpg` in the evidence folder.

**M1 is feature-complete.** The last step is yours: play it and tell me what
feels wrong.

## Play it yourself

```
cd C:\code\triangulum\game
cargo run --release
```

You start walking on the hills. Click to capture the mouse (Esc releases).
WASD walk, Space jump (swim up in water), Shift sprint, G walk/fly, left
mouse breaks (hold to repeat), right mouse places, 1-9 pick the block, R
drops you to the ground, F hides water, H hides the HUD, V vsync, P scenic
places, Q saves and quits, F1 opens the look panel, F10 records a replay
(see below). Edits save
automatically to `game/saves/Neisor/`.
Days are 24 minutes: `[` and `]` slow down and speed up time (up to
3,600×), and `--time 17:30` starts at golden hour.

Try other worlds (each keeps its own save):

```
cargo run --release -- --planet-size 832 --save saves/earth             # the Earth analogue
cargo run --release -- --vscale 0.05                                    # 1/20
cargo run --release -- --vscale 0.125 --height 4096 --n 131072          # 1/8
cargo run --release -- --block-size 0.5 --n 262144 --height 4096 --vscale 0.05 --save saves/half   # half-metre blocks
```

`--height` keeps the planet's size in half-heights and centres the sea, so a
taller world is a bigger planet unless `--n` says otherwise.

Hold Ctrl while walking to sneak: you won't walk off edges, so you can
back out over a gap and place blocks under you. Known issues: ripple
patterns on distant steep terrain, and gully stripes that can look combed
on gentle hills (Andrew's call). `--lod-factor 10` shows finer distant
detail (still about 500 fps).

## In progress

**Round 2:** Andrew's second list, matched to his photos and recordings in
`docs/missions/2026-10-31-round2.md`: the crashes, great-circle movement,
sliding powder, water flicker, and the terrain work (real mountains, rivers
carved into the land, boundaries rough at every scale, sea level in the middle
and a planet-size slider). Tasks are in `PLAN.md`.

- **C1, merged:** minimising no longer kills the game, the wheel has no ceiling
  (1 cm/s up, past the sun if you hold it), and WASD follows great circles over
  both poles. Every panic now leaves a file in `game/out/crash/`. Recordings
  from before this refuse to play and say why; their markers and reports stand.
  The powder launch crash was **not reproduced**: ten real launches ran clean.
  Three launch failures that look like a crash are fixed, but none is known to
  be what Andrew saw. Findings: `docs/missions/C1-window-findings.md`.
- **C2, merged:** powder slides over a ledge instead of teleporting, and there
  is never a second copy of it. Water no longer blinks as it spreads (351
  blinking cells in Andrew's recording under the old rules; 0 flickers in the
  new test of that scene, whose stats line now counts them). The lone cell no
  longer fights the ground: it was level 1, not 0, and its surface lay exactly in
  the floor. Recordings from before this refuse to play.
  Nobody has watched the slide in a real window yet: harness frames and video
  only. Findings: `docs/missions/C2-motion-findings.md`.
- **T1, merged:** sea level is in the middle of the world, and the coarsest voxel
  is half the world height. Planet size is a slider in F1 > Terrain, from 128 to
  1024 half-heights, with an Earth analogue button at 832 (R 589 km). It builds a
  preview world, so your own world is safe, and *Save look* keeps the size for new
  worlds. Walking, digging, water, saves and replays pass at 128, 832 and 1024
  on this build, and at 256 and 512 in Astra's own runs before the merge. Nobody
  has moved the slider by hand. Old saves and recordings are refused and left
  untouched. Findings: `docs/missions/T1-planet-findings.md`.
- **Sky fixed, merged:** the hard line across the sky when you were high up is
  gone, and so is the black band over the horizon. The sky treated eye level as
  the horizon. Ground and orbit views barely change
  (`docs/missions/2026-10-31-round2-evidence/sky-line-before-after.jpg`).
- **T2, merged:** real mountains. Peaks stand up to about 800 m on the belts,
  where the highest ground on the planet was 143 m, with ridges, V-shaped valleys
  and gorges. Rivers lie in valleys and lakes in basins: banks where the ground
  rose towards the channel fell from 51.3% to 4.7%. Hills now differ from place
  to place. Local features stay in metres on any planet, and global ones stretch
  with it (`DESIGN.md`). Old saves are refused again: generator 6. Nobody has
  walked this in a real window. Findings: `docs/missions/T2-relief-findings.md`.
- **A rare crash on exit, not explained:** now and then the program dies with an
  access violation (`0xC0000005`) as it closes, after everything is finished and
  saved. It's been seen about four times across this round's hundreds of runs,
  including on the old build, so it isn't new. All were headless test runs, and
  nothing was lost in any of them. The cause is unknown, because a crash like this
  doesn't go through the panic handler and so leaves no file in `game/out/crash/`.

**The first bug round, all merged:** Andrew's first list, 11 items plus
Austin's annotation tool, matched to his recordings in
`docs/missions/2026-10-30-bug-round.md`.

| Mission | Bugs | Who | State |
|---|---|---|---|
| B1 water surfaces | #1 water under a solid, #10 dip at the edge, review 3 #8 | Opus | merged; hairlines passed to B2 |
| B2 shores | #2 lakes from space, #4 ocean bands, #5 beaches | Opus | merged; the lake step between detail levels is open |
| B3 picking | #3 raycasting at cube seams and corners | Astra | merged |
| B4 flow | #7 sliding powder and water, #9 level-1 film | Opus | merged |
| B5 tools | #6 colourised mesh view, #11 map, screenshots and teleport | Opus | merged |
| Review 4 | M2c, M3a, hardening, M3b: 13 findings, 3 of them P1 (`docs/reviews/2026-10-31-engine-review4-astra.md`) | Astra | done, not yet re-verified |

| B6 reports | Annotated bug reports for screenshots and recordings, review 4 #8 | Opus | merged |
| R4 hardening | Review 4's other findings, the save-safety ones first | Astra | merged |
| B7 powder levels | #8: an experiment, powder with levels and an angle of repose, off by default | Opus | merged |
| S1 saves | Saves off the main thread, removing the autosave hitch | Astra | merged |

Next, once Andrew has weighed in: M3c (non-full blocks) needs his look
decisions, and underground darkness needs a lighting design.

## Waiting on you

- **Your old save won't load.** The terrain changed, so `saves/Neisor` is
  refused. It's untouched. Start a new world:
  `cargo run --release -- --save saves/neisor2`.
- **Try the new keys in a real window.** T, P, Shift+P, M, Shift+M, F9, Esc,
  the mouse wheel on the map, the annotator's pen, and clicking at cube corners
  were all driven by test events, never by real hands. Tell me if any of them
  misbehave.
- **Try leaving the window (C1).** No test has had real focus. Alt-Tab away: the
  pointer should be free at once. Come back by Alt-Tab and by clicking: the
  first click should capture the mouse and break nothing. Click the taskbar
  button twice. And `--powder levels --save saves/Neisor` should show a box
  saying why it won't start; nobody has seen that box yet.
- **Andrew, if the powder crash comes back:** tell me what you typed, and send
  the newest file in `game/out/crash/`.
- **Andrew's calls from C1:** the mouse stays free until a click after you leave
  the window (the other way, Alt-Tab recaptures it but clicking back breaks a
  block); the wheel's floor is 1 cm/s (it was 1 m/s); and level flight now stays
  level, so flying at the horizon from orbit circles the planet. To leave, look
  up or hold Space.
- **Andrew's calls from C2.** The first one matters most, because it narrows his
  own rule:
  1. **The ledge rule (`DESIGN.md`, "Waiting for Andrew's OK").** To stop the
     blinking, a sheet of level-1 water near a ledge now holds and dries in 10 to
     30 s while thicker water drains through it. Before, it all went over in about
     3 s, blinking. A wet patch lingers where a puddle was, and a waterfall's lip
     cell stays dry.
  2. The slide's feel: 0.2 s, fast off the edge, then falling. One number.
  3. How faint a film should be: 30% of a lake's opacity below 0.12 of a voxel.
     Both are in the shader, and could be panel dials.
- **Andrew's calls from T1:** is 832 what you meant by an Earth analogue? And how
  do the air and fog look at big sizes: they now keep the same proportions as
  today's planet, so at 2 km up a big planet still has a blue sky.
- **Andrew's calls from T2.** The first one matters most:
  1. **Mountains on big planets.** At 832 the bake's own heights take up most of
     the 1,024 m above the sea, so the highest belts become rolling high country
     with about 190 m of relief, not 800 m peaks. See
     `docs/missions/T2-relief-evidence/sizes_832_after.jpg`. Without this they
     were a flat cap with pits in it. A taller world would give them room:
     `--height 4096 --n 851968` is the same planet with twice the height.
  2. Peak height (`MOUNT_M`, 640, peaks about 800 m), how sharp (`RIDGED_DOWN_TO`),
     hills, and valley shape. The names are in the findings.
  3. **Lakes lowered to their inflow:** 415 of 771 moved, mostly by about a metre,
     and 42 by over 10 m. The other way is a levee, or a gorge running up to the
     lake.
  4. Mesa steps are now 40 m at most, and stay out of the belts.
  5. High plateaus are frozen at every latitude. That's the bake's climate at
     that height, not a bug in the snow line.
- **Austin: Visual Studio needs repairing** when a restart suits you: restart,
  then tick *Desktop development with C++* in the Visual Studio Installer, then
  delete `game/.cargo/config.toml`. Until then the game builds from a linker
  on `E:/triangulum-build/`.
- **Andrew's answers to the bug round's list (2026-10-31):** water evaporates;
  powder levels stay out of terrain generation for now; everything else stays
  as it is. `DESIGN.md` has them.
- **Andrew: the look panel is yours** (F1). Most of the art notes below
  are now dials he can turn himself: atmosphere height, night brightness,
  dusk colours, water colour, grass sides. *Save look*, then commit the
  two files or send them. The panel has only been seen at this machine's
  150% display scaling; tell me if it's too small or too big on yours.
- **One replay across PCs.** F10 works: all five recordings from
  2026-10-30 played back exactly on this machine. One recorded on Andrew's
  PC and played on Austin's tells us whether replays survive a different
  machine.
- **Watch the M1 video:** `docs/media/m1-showcase.mp4` (85 s; stills in
  `docs/media/m1-showcase-stills/`). Things in it that are Andrew's call:
  - **The atmosphere is thin.** It's scaled to the 90 km planet, so the sky
    turns toward space above about 1.5 km, and 1/8 mountains reach into
    it. Whatever vertical scale he picks, the atmosphere's height should
    follow; it's one setting.
  - **Oceans from orbit** are now an even grey-blue; the deep blue before
    was a bug (space showing through). Bluer water is a colour choice.
  - **Dusk and night are weak,** and golden hour is subtle.
  - **Underground isn't dark yet.** (Green tunnel ceilings are fixed:
    grass now has dirt sides and bottom.)
  - **Distant terrace stripes** ripple on steep slopes far away.
- **Water behaviours for Andrew to judge** (try pouring and digging):
  1. Flow speed: one block per tick (20 a second); it can be slowed.
  2. Settled water may keep a slight lean (up to 1 level per block), so a
     pit dug beside the sea keeps a 7 cm dip. That follows from his rule;
     flattening it would need a second rule.
  3. One poured block on flat ground spreads into a 4 mm sheet over up to
     255 blocks. There's no evaporation or minimum depth.
  4. Large bodies settle slowly: a 100 m dam break takes minutes.
  5. Rivers stay frozen at their generated levels (costing nothing) until
     disturbed; then that reach flows by his rule, and its steps and
     waterfalls dissolve into a trickle fed from upstream.
- **Andrew:** with 24-minute days, do the July calendar numbers still stand
  (7-day lunar month, 84-day year)? No rush.
- **Andrew:** pick a grid by eye if he wants something other than the
  recommended smooth equal-area one (grid page linked above).

## Decided

See `DESIGN.md`: the smooth equal-area grid (`EqualAreaCobe`), planet
131,072 columns per face (R ≈ 90.6 km), Andrew's 256-level water rule, and
from 2026-10-30: textures are 16×16, and the scale stays where it is for now
(1 m blocks, world height 2,048, vertical scale 1/70).
