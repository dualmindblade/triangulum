# M3b: add blocks and paint their textures in the game (Opus, 2026-10-25)

Branch `worktree-agent-a1a7e5673c85056ef`. Evidence in `M3b-evidence/`.
Andrew's how-to is in `game/README.md` ("Adding a block", "Painting a
texture"); the engineering is "Blocks and textures (M3b)" there.

## What it adds

- **Add a block** (F1, Blocks, *Add a block...*): a name, a colour, and
  *solid* or *falls like sand*. It takes the next id and is written to
  `blocks.json` at once, so a save that holds it always finds it. Ids are
  append-only; *hidden* takes a block off the hotbar list instead of
  deleting it. Blocks Andrew adds can be renamed (the old name goes into
  `"was"`); the shipped ones keep their names, because the generator and
  the scripts find them by name. Switching *solid* / *falls like sand*
  shows what it does first and waits for *Yes*.
- **The hotbar**: nine key boxes in the panel, and *Put it there* on a
  block. Saved in `blocks.json` (`"hotbar"`) at once; the game starts with
  it. The play harness and replays keep their own (scripts press 1-9
  expecting the default); a new harness command `hotbar A,B,...` sets it in
  a script.
- **Textures**: an optional picture per face (top, sides, bottom) in
  `game/assets/textures/`, referenced from `blocks.json` (`texture`,
  `top_texture`, `side_texture`, `bottom_texture`). One setting,
  `"texture_size": 16`, sizes every picture; any PNG is resampled to it.
  The GPU holds them as one texture array with mip levels averaged in
  linear light. The shader tiles a texture once per block (texture
  coordinates in blocks), on greedy quads and coarse voxels alike, and
  samples only for textured faces: with no textures the frames are the
  pre-M3b frames bit for bit.
- **The editor**: a pixel grid for the chosen faces (*every face*, *top*,
  *sides*, *bottom*), pencil, fill, pick colour (also a right click), a
  colour chooser with recent colours, undo (Ctrl+Z), *Start from the
  block's colour*, *Import PNG...* (the Windows file dialog; any size, cut
  to the middle square), dropping a PNG on the window, *Remove texture*,
  and three examples. Every change shows on the blocks in the world and on
  the hotbar at once. *Save look* writes the PNGs and `blocks.json`.
  Painting an example (or a picture another block uses) makes the block
  its own copy.
- **Hot reload**: a texture PNG edited in a paint program shows within a
  second; a `blocks.json` changed on disk (a pull) brings new blocks,
  renames, falling, hiding, the hotbar, colours and textures in live, and
  is refused whole, with the reason, if it renumbers or drops a block.
- **Examples**: `game/assets/textures/examples/` (stone, dirt, grass top)
  with a README saying they are placeholders. Not enabled. Each averages
  to its block's colour, so far away a block wearing one looks unchanged.
- **Window driving** (`game/tools/window.ps1`): start, attach by process
  id, click, drag, scroll, type, keys, relative mouse look, screenshot of
  the client area, stop with Q. Documented in the README.

## Saves and replays when a block is added

How they treated the registry before: the save header lists (id, name)
for every block and accepted a registry that extends it already, but a
rename looked like a renumbering; the simulation's `rules_digest` hashed
every block's name and flags and went into `Sim::state_hash`, so adding
any block changed the end hash of every replay recorded before.

Now:
- **Header** (`world/save.rs`, one hunk in `Header::check` and one new
  function `extend_header_blocks` called from `flush`; nothing else in
  `save.rs`): a saved name may be the block's current name or one in its
  `"was"`; blocks appended since the header was written are added to it on
  the next save. A renumbering is still refused, and the message now says
  how a hand rename should keep the old name.
- **Rules digest** (`sim/mod.rs`, `rules_digest_upto`, `rules_extend`, and
  three lines in `state_hash`): a block's identity is its first name; the
  state hash covers the rules of the blocks the state can hold (the
  generator's, the highest id an edited chunk or a falling block holds).
  For the shipped blocks that is exactly the old digest, so existing
  replays still match. `session.rs` (one line): the "other block rules"
  note accepts a registry that extends the saved one.
  `worldgen/mod.rs` gains `Generator::max_block_id`.
- **What a flag change does**, as the panel says: saves keep loading;
  blocks of that kind already in the world fall only when something next
  to them changes; replays recorded before report a difference (the hash
  covers the changed rule). Changing the hotbar or a block's falling ends a
  replay recording first, like a terrain preview.

These touch Fable's files (`save.rs`, `sim/mod.rs`, `session.rs`) in small
separate hunks; please check them against M2-hardening when merging.

## Evidence

- **The real window, driven** (`live_01..17`): F1, Blocks, *Add a block*,
  typed "red brick", *Add block* (written to blocks.json at once, block 19),
  *Put it there* on key 1 (the hotbar saved), *Start from the block's
  colour*, a brick pattern drawn with pencil drags, the colour chooser,
  fill, undo, pick colour, pencil dots; bricks built in the world with the
  painted texture; the mortar refilled while looking at them (the world
  updated live); *Import PNG...* through the Windows file dialog (a 240 x
  160 picture became 16 x 16 on the blocks and the hotbar), undo of the
  import; *Save look* (PNG and blocks.json written); the PNG edited from
  outside the game (the world, the editor and the hotbar changed within a
  second, with a note); *falls like sand* clicked (the warning, then
  Cancel); renamed to clay brick (`"was": ["red_brick"]`); quit with Q;
  restarted on the same save: it loads, the bricks are there, key 1 is
  the textured clay brick.
- **Frames unchanged with no textures**: `m1-done`, `m3a-look` and
  `m3a-tunnel` rendered with the pre-M3b build and this one: every 3D pixel
  identical; the only differences are the HUD's "tick cost" timing text.
- **A painted block near and far** (`far_painted_brick_sheet.jpg`): the
  live session's bricks seen from 12 m, 100 m, 300 m and 1 km, against the
  same bricks flat at the picture's average colour: at 1 km 8 pixels
  differ by at most 1 level; at 300 m 10 pixels by at most 6.
- **Every block textured** (`every_block_textured_vs_flat.jpg`, the bench
  look from `make_example_textures.py --every-block`): at 1 km the frame
  is within 6 levels of the flat look (mean 0.47); from 1.5 km up
  (`m3a-look` sky shot) within 1 level.
- **No shimmer**: two 1 km frames 0.3 m apart change by a mean of 0.235
  levels with every block textured and 0.156 flat; pixels changing by more
  than 8 levels (geometry edges moving): 3,608 textured, 3,625 flat.
- **Orientation** (`orientation_check.jpg`): an asymmetric "F" texture
  reads upright and unmirrored on all four sides of a pillar; on tops it is
  rotated, never mirrored.
- **Tests**: BENCH_TESTS
- **Play scripts**: PLAY_SCRIPTS
- **Bench** (provisional: two other agents' `triangulum` processes and
  cargo were running): BENCH_TABLE

## Not verified

- Readability at 100% display scaling (this machine runs at 150%).
- Dropping a PNG on the window: the code path is egui's dropped files,
  and the import it calls was driven through the file dialog, but no drop
  was performed (it needs a real drag from Explorer).
- The file dialog opening in the textures folder: the session's dialog
  opened in Documents (the folder was a relative path); fixed to an
  absolute path afterwards but not seen again.
- *Make this the default* and terrain previews with textures on, and
  `texture_size` 32 in the running game (unit-tested resampling only).
- Andrew's machine, and a pull of someone else's blocks.json while playing
  (the merge is unit-tested).

## Open decisions (for Andrew)

- **Texture size**: 16 x 16 now, one setting (`"texture_size"` in
  blocks.json). 32 is finer and four times the painting. His call.
- **The hotbar is in blocks.json**, so it is shared when he commits it.
  If he and Austin want their own hotbars, it could move to a per-machine
  file.
- **See-through blocks and new liquids**: not in the game. Glass and
  leaves need a cut-out drawing pass (a third draw list in the streamer);
  a second liquid needs the water rules to know it (they turn any flowing
  liquid into water). The panel says so.

## Feedback

- **The window tool works, with care.** Screenshots copy the screen: copy
  only the game's client area (`Save-GameShot`). Once I captured the whole
  screen to see a file dialog and it showed private folder names from the
  Documents folder; I deleted it at once and it is not in the evidence.
  .NET resolves relative paths against the process's directory, not
  PowerShell's location (`Save-GameShot` now makes them absolute). Windows
  only lets `SetForegroundWindow` through after an Alt tap. Relative mouse
  look through `mouse_event` worked once and then did nothing until the
  mouse was captured again; strafing with the keys was the reliable way to
  frame a view. One PowerShell command per step, attaching by process id,
  made a long session easy to steer.
- **A one-pixel dark line** runs down the bottom of some hills frames (for
  example `far_painted_brick_near_12m.jpg`, x 912, and the live shots after
  strafing). The pre-M3b build draws it identically at the same pose, so it
  is an existing crack between terrain quads showing a dirt side, not M3b.
  Worth a look in the streaming or mesher work.
- **Bench contamination**: other agents' processes were running throughout;
  a "wait for a quiet minute" helper (M3a's suggestion) is still missing.
