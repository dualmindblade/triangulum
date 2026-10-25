# M3b: add blocks and paint their textures in the game (Opus, 2026-10-25)

Branch `worktree-agent-a1a7e5673c85056ef`, merged with main after
M2-hardening (8fc787b); everything below was re-run on the merged tree.
Evidence in `M3b-evidence/`.
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
  function `extend_header_blocks` called from `flush_with`, on M2-hardening's
  `SaveFs` path with its directory sync; nothing else in `save.rs`): a saved name may be the block's current name or one in its
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
separate hunks; the merge with M2-hardening conflicted only in `flush`
(resolved onto its `SaveFs` calls) and in `main.rs` (the bake identity now
comes from the loaded bake). M2-hardening's `RULES_VERSION` (2) in replay
headers is independent of this: a replay from before M2-hardening is
refused by version, whatever blocks exist.

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
  `m3a-tunnel` rendered with main's build (M2-hardening, no M3b) and with
  the merged M3b build: every pixel outside the HUD's stats box identical
  in all 18 frames; inside it only the "tick cost" timing text differs.
- The far, shimmer and orientation checks below ran on the M3b build
  before the merge; the merge touched no rendering code.
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
- **Tests**: `cargo test --release` on the merged tree: 165 passed, 0
  failed (14 ignored, as before). New: the registry (add, rename, fall,
  hide, write, merge, refusals), blocks.json with textures round-trip,
  textures (sRGB, linear-light mips, resampling, PNG round trip, fill,
  hot reload of a texture file, paint/save/revert), the editor (lines,
  undo), the workshop (painting an example makes a copy and Save look
  writes it; blocks.json edited on disk brings a block in live; add,
  rename, fall and hotbar through a real session), and
  `compat_tests.rs`: a save and a replay made before a block was added
  load and replay exactly after it, live and after a restart, a renamed
  block's save still loads, and a block that now falls is still caught.
  With the state-hash change reverted, the replay part fails ("the end
  state hash DIFFERS"), so the test guards it.
- **Play scripts** on the merged tree, all pass: m1-done (94 asserts),
  tunnel-wall-reload (18), edge-corner-flight (17), m2a-sand (95),
  m2b-water (97), m2b-boundary (44), m2b-river (41), m2c-replay (29),
  m2c-video-driver (21), m3a-tunnel (20), m3a-look; and the new
  `m3b-hotbar.play` (11 asserts; `--blocks scripts/m3b-look/blocks.json`):
  the live session's clay brick on key 1 through the new harness command
  `hotbar`, built into a wall (`m3b_hotbar_wall_near.jpg`).
- **Across builds** (`compat-main-build.txt`): a save and a replay
  recorded by main's build (M2-hardening, no M3b) load and replay exactly
  in this build with a block added to blocks.json: 7 of 7 checks and the
  end state hash match.
- **Bench, provisional** (another session's game window and cargo were
  running; `bench-merged-provisional.txt`): three interleaved runs each of
  main's build, this build with no textures, and this build with every
  solid block textured (20 pictures). Flight average fps, best of three:
  1,094 / 1,053 / 1,046; 1% low, best of three: 420 / 330 / 366 (the 60
  fps target is 16.7 ms a frame; the worst frame in the best runs is 5.0 /
  8.7 / 5.4 ms). The wall clock swung by up to 40% between runs of the same
  build, so those differences are noise. The GPU time is steadier: flight
  GPU average 0.37-0.38 ms on main, 0.39-0.40 ms on this build with no
  textures, 0.40 ms with every block textured. So textures cost nothing
  measurable on top, but the texture path itself (two more interpolated
  values and the derivatives in the fragment shader) costs about 0.02 ms of
  GPU a frame even with no textures, about 5% of a 0.38 ms frame. The
  frames themselves are unchanged. The first bench round, before the merge,
  was too contaminated to use and is not reported.

## Found by driving the window, fixed after the session

- After *Add block* the panel did not scroll to the new block (it does
  now). The file dialog opened in Documents, not the textures folder (a
  relative path; now absolute). A rename left the old name in the file's
  hotbar (it worked through `was`; now the hotbar is renamed too). The
  hotbar caption showed `red_brick` (now "red brick").
- From review: the "fixed name" rule used the compiled-in blocks.json, so
  Andrew's own blocks would have become un-renamable once committed (now
  the generator's blocks); `default_hotbar` skipped hidden blocks, so
  hiding a shipped block would have moved keys 1-9 under the play scripts
  (now fixed whatever is hidden).

## Not verified

- A clean bench: the machine was never idle (see above). Whether the
  0.02 ms GPU cost with no textures is real needs an idle run; if it is,
  a separate pipeline for textured faces would remove it.

- Readability at 100% display scaling (this machine runs at 150%).
- Dropping a PNG on the window: the code path is egui's dropped files,
  and the import it calls was driven through the file dialog, but no drop
  was performed (it needs a real drag from Explorer).
- The file dialog opening in the textures folder: the session's dialog
  opened in Documents (the folder was a relative path); fixed to an
  absolute path afterwards but not seen again.
- Clicking *Yes, make it fall* in the window (the warning and *Cancel*
  were driven; the change itself is tested through a session).
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
- **Bench contamination**: other sessions' processes ran throughout (cargo,
  and a `triangulum` game window started without arguments). I tried a
  "wait for a quiet minute" check (no triangulum/cargo/rustc, CPU < 12%)
  before each run; it never passed because idle leftover processes and a
  game window idling at vsync count as busy. A useful version needs to
  know which processes belong to the run, or a lock the agents share.
