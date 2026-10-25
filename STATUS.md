# Status

*Updated 2026-10-25 by Claude (Opus).*

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
  **1,134 fps average, 611 fps for the worst 1% of frames, worst frame
  6.0 ms** (everything through M3b, 2026-10-25). The target is 60 fps
  (16.7 ms).
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
cargo run --release -- --vscale 0.05                                    # 1/20
cargo run --release -- --vscale 0.125 --height 4096 --sea-level 1536    # 1/8
cargo run --release -- --block-size 0.5 --n 262144 --height 4096 --sea-level 1024 --vscale 0.05 --save saves/half   # half-metre blocks
```

Hold Ctrl while walking to sneak: you won't walk off edges, so you can
back out over a gap and place blocks under you. Known issues: ripple
patterns on distant steep terrain, and gully stripes that can look combed
on gentle hills (Andrew's call). `--lod-factor 10` shows finer distant
detail (still about 500 fps).

## In progress

Nothing: paused for the week. The next steps are in
`docs/handoffs/2026-10-25-week-end.md`.

## Waiting on you

- **Andrew: the look panel is yours** (F1). Most of the art notes below
  are now dials he can turn himself: atmosphere height, night brightness,
  dusk colours, water colour, grass sides. *Save look*, then commit the
  two files or send them. The panel has only been seen at this machine's
  150% display scaling; tell me if it's too small or too big on yours.
- **Andrew: texture size** (open decision, M3b). Block textures are
  16 x 16 pixels. One setting, `"texture_size"` near the top of
  `game/assets/blocks.json`, makes them 32 x 32: finer, and four times the
  painting. Pictures of the other size are resized when read, so it can
  change later without losing anything.
- **Try F10 once each.** Press F10, play a minute (dig, pour water, G, F,
  `[` `]`, P, R, alt-tab), press F10 again, and send the file from
  `game/replays/` (how: "Reporting a bug" in `game/README.md`). No agent
  can press real keys, so the key mapping is untested. One recorded on
  Andrew's machine and played back on Austin's tells us whether replays
  survive a different PC.
- **Watch the M1 video:** `docs/media/m1-showcase.mp4` (85 s; stills in
  `docs/media/m1-showcase-stills/`). Things in it that are Andrew's call:
  - **Vertical scale.** At 1/70 the highest range is a white plain with
    contour stripes; at 1/8 it's real mountains (last shot of the video).
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
- Block size (1 m or ½ m) gets decided after you've played M1.

## Decided

See `DESIGN.md`: the smooth equal-area grid (`EqualAreaCobe`), planet
131,072 columns per face (R ≈ 90.6 km), Andrew's 256-level water rule.

## Under review

- World height: 2,048 blocks suggested (512 felt small); the engine
  benchmark measures what height costs.
- Vertical scale: between 1/70 (Earth's slopes) and 1 (Earth's heights);
  Andrew decides from renders at 1/70, 1/20 and 1/8, coming with M1a.
